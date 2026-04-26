import sys
import os
from pathlib import Path

from PySide6.QtCore import QUrl, Qt
from PySide6.QtWidgets import QApplication, QMainWindow, QVBoxLayout, QWidget
from PySide6.QtWebEngineWidgets import QWebEngineView
from PySide6.QtWebChannel import QWebChannel

from app.bridge import Bridge

WWW_DIR = Path(__file__).resolve().parent / "app" / "www"
INDEX_PATH = WWW_DIR / "index.html"


class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("YouTube Summarizer")
        self.resize(1200, 750)
        self.setMinimumSize(900, 500)

        central = QWidget()
        self.setCentralWidget(central)
        layout = QVBoxLayout(central)
        layout.setContentsMargins(0, 0, 0, 0)

        self.webview = QWebEngineView()
        layout.addWidget(self.webview)

        self.channel = QWebChannel()
        self.bridge = Bridge()
        self.channel.registerObject("bridge", self.bridge)

        self.webview.page().setWebChannel(self.channel)
        self.webview.setUrl(QUrl.fromLocalFile(str(INDEX_PATH.resolve())))

        self.webview.page().fullScreenRequested.connect(self._handle_fullscreen)

    def _handle_fullscreen(self, request):
        request.accept()
        if request.toggleOn():
            self.showFullScreen()
        else:
            self.showNormal()

    def closeEvent(self, event):
        self.webview.page().profile().clearHttpCache()
        super().closeEvent(event)


def _detect_platform() -> str:
    explicit = os.environ.get("QT_QPA_PLATFORM")
    if explicit:
        return explicit
    if os.environ.get("WAYLAND_DISPLAY"):
        return "wayland"
    if os.environ.get("DISPLAY") or sys.platform == "darwin":
        if sys.platform == "linux":
            if not _has_libxcb_cursor():
                print("⚠️  libxcb-cursor0 wird benötigt aber ist nicht installiert.")
                print("   Installiere es mit: sudo apt install libxcb-cursor0")
                print("   Starte mit offscreen-Fallback...")
                return "offscreen"
        return "xcb"
    return "offscreen"


def _has_libxcb_cursor() -> bool:
    try:
        import ctypes.util
        return ctypes.util.find_library("xcb-cursor") is not None
    except Exception:
        return True  # assume it's there on non-Linux


def main():
    platform = _detect_platform()
    os.environ["QT_QPA_PLATFORM"] = platform

    app = QApplication(sys.argv)
    app.setApplicationName("YouTube Summarizer")
    app.setOrganizationName("youtube-summarizer")

    window = MainWindow()
    window.show()

    sys.exit(app.exec())


if __name__ == "__main__":
    main()
