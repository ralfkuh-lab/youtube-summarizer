#!/usr/bin/env bash

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

missing_pkgs=()
missing_sys=()

# ── Python-Check ──────────────────────────────────────────────
if ! command -v python3 &>/dev/null; then
    echo -e "${RED}Python 3 ist nicht installiert.${NC}"
    echo "Bitte installiere Python 3.9+ von https://python.org"
    exit 1
fi

PY_VER=$(python3 -c "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')")
PY_MAJ=$(echo "$PY_VER" | cut -d. -f1)
PY_MIN=$(echo "$PY_VER" | cut -d. -f2)
if [ "$PY_MAJ" -lt 3 ] || { [ "$PY_MAJ" -eq 3 ] && [ "$PY_MIN" -lt 9 ]; }; then
    echo -e "${RED}Python $PY_VER gefunden, aber mindestens 3.9 wird benotigt.${NC}"
    exit 1
fi
echo -e "${GREEN}Python $PY_VER ✓${NC}"

# ── System-Pakete (Linux) ─────────────────────────────────────
if [ "$(uname -s)" = "Linux" ]; then
    if ! python3 -c "import ctypes.util; exit(0 if ctypes.util.find_library('xcb-cursor') else 1)" 2>/dev/null; then
        if ! dpkg -s libxcb-cursor0 &>/dev/null; then
            missing_sys+=("libxcb-cursor0")
        fi
    fi
fi

# ── Python-Pakete ─────────────────────────────────────────────
required_pkgs=(
    "PySide6"
    "youtube_transcript_api"
    "sqlalchemy"
    "httpx"
)

for pkg in "${required_pkgs[@]}"; do
    if python3 -c "import $pkg" &>/dev/null; then
        echo -e "${GREEN}$pkg ✓${NC}"
    else
        echo -e "${YELLOW}$pkg ✗${NC}"
        missing_pkgs+=("$pkg")
    fi
done

# ── Konfiguration ─────────────────────────────────────────────
if [ ! -f "config.json" ]; then
    echo ""
    echo -e "${YELLOW}Keine config.json gefunden.${NC}"
    if [ -f "config.example.json" ]; then
        cp config.example.json config.json
        echo -e "${GREEN}config.example.json → config.json kopiert.${NC}"
        echo -e "${YELLOW}Bitte API-Key in config.json eintragen!${NC}"
    fi
fi

# ── Installation anfragen ─────────────────────────────────────
needs_install=false

if [ ${#missing_sys[@]} -gt 0 ]; then
    needs_install=true
    echo ""
    echo -e "${YELLOW}Fehlende System-Pakete:${NC} ${missing_sys[*]}"
    if ! command -v sudo &>/dev/null; then
        echo -e "${RED}sudo ist nicht verfugbar. Bitte manuell ausfuhren:${NC}"
        case "$(uname -s)" in
            Linux)
                echo "  sudo apt install ${missing_sys[*]}"
                echo "  # oder fur andere Distributionen das Aquivalent" ;;
        esac
    fi
fi

if [ ${#missing_pkgs[@]} -gt 0 ]; then
    needs_install=true
    echo ""
    echo -e "${YELLOW}Fehlende Python-Pakete:${NC} ${missing_pkgs[*]}"
fi

if $needs_install; then
    echo ""
    read -r -p "Sollen die fehlenden Abhangigkeiten jetzt installiert werden? [Y/n] " answer
    if [ "$answer" != "n" ] && [ "$answer" != "N" ]; then
        # System-Pakete
        if [ ${#missing_sys[@]} -gt 0 ]; then
            echo ""
            echo -e "${CYAN}Installiere System-Pakete...${NC}"
            if command -v sudo &>/dev/null; then
                sudo apt-get update -qq 2>/dev/null || true
                sudo apt-get install -y "${missing_sys[@]}" || {
                    echo -e "${RED}System-Paket-Installation fehlgeschlagen.${NC}"
                    echo "Bitte manuell ausfuhren: sudo apt install ${missing_sys[*]}"
                }
            else
                echo -e "${RED}Kein sudo verfugbar. Bitte manuell ausfuhren:${NC}"
                echo "  sudo apt install ${missing_sys[*]}"
            fi
        fi

        # Python-Pakete
        if [ ${#missing_pkgs[@]} -gt 0 ]; then
            echo ""
            echo -e "${CYAN}Installiere Python-Pakete...${NC}"
            if [ -n "$VIRTUAL_ENV" ]; then
                pip3 install "${missing_pkgs[@]}" || {
                    echo -e "${RED}Python-Paket-Installation fehlgeschlagen.${NC}"
                    exit 1
                }
            else
                pip3 install --break-system-packages "${missing_pkgs[@]}" || {
                    echo -e "${RED}Python-Paket-Installation fehlgeschlagen.${NC}"
                    echo "Versuche mit --user:"
                    pip3 install --user "${missing_pkgs[@]}" || exit 1
                }
            fi
        fi
        echo ""
        echo -e "${GREEN}Alle Abhangigkeiten installiert.${NC}"
    else
        echo "Starte trotzdem..."
    fi
else
    echo ""
    echo -e "${GREEN}Alle Abhangigkeiten sind vorhanden.${NC}"
fi

# ── Start ─────────────────────────────────────────────────────
echo ""
echo -e "${CYAN}Starte YouTube Summarizer...${NC}"
echo ""
exec python3 main.py
