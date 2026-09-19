//! Shell-Auswahl und Maskierung fuer das uebergebene Kommando.
//!
//! Grundsatz: jeder aus der Konfiguration oder dem Video stammende Wert wird
//! genau einmal maskiert und danach nie wieder auf Platzhalter geprueft.

/// Wirksame Shell der Kommandozeile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    /// sh, bash, zsh.
    Posix,
    Fish,
    Powershell,
}

impl Shell {
    /// Loest den Konfigurationswert auf; `auto` ist PowerShell unter Windows,
    /// sonst POSIX.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::platform_default()),
            "posix" => Ok(Self::Posix),
            "fish" => Ok(Self::Fish),
            "powershell" => Ok(Self::Powershell),
            _ => Err("Unbekannte Shell".to_string()),
        }
    }

    pub fn platform_default() -> Self {
        if cfg!(windows) {
            Self::Powershell
        } else {
            Self::Posix
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Posix => "posix",
            Self::Fish => "fish",
            Self::Powershell => "powershell",
        }
    }
}

/// Steuerzeichen laut Spezifikation: `\0`-`\x1f`, `\x7f` sowie die
/// Zeilentrenner U+2028 und U+2029. Die C1-Bereichszeichen (`\x80`-`\x9f`)
/// sind bewusst nicht enthalten.
pub fn has_forbidden_control(value: &str) -> bool {
    value
        .chars()
        .any(|ch| ch < ' ' || ch == '\u{7f}' || ch == '\u{2028}' || ch == '\u{2029}')
}

/// Maskiert einen Wert fuer die angegebene Shell.
///
/// Lehnt Steuerzeichen selbst ab; die Aufloesung prueft die Werte vorher mit
/// ihrem Namen und liefert dort den sprechenden Fehler.
pub fn shell_quote(value: &str, shell: Shell) -> Result<String, String> {
    if has_forbidden_control(value) {
        return Err("Ungültige Zeichen im Wert".to_string());
    }
    Ok(match shell {
        Shell::Posix => format!("'{}'", value.replace('\'', "'\\''")),
        Shell::Fish => format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'")),
        Shell::Powershell => format!("'{}'", value.replace('\'', "''")),
    })
}
