import sys
import os
import threading
import socket
from pathlib import Path
from http.server import HTTPServer, SimpleHTTPRequestHandler

from PySide6.QtCore import QUrl, Qt
from PySide6.QtWidgets import QApplication, QMainWindow, QVBoxLayout, QWidget
from PySide6.QtWebEngineWidgets import QWebEngineView
from PySide6.QtWebChannel import QWebChannel
from PySide6.QtWebEngineCore import QWebEngineSettings

from app.bridge import Bridge

WWW_DIR = Path(__file__).resolve().parent / "app" / "www"


def _find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _start_http_server(port: int, automation: bool = False) -> HTTPServer:
    import os as _os
    _os.chdir(str(WWW_DIR))

    if automation:
        from app.automation import AutomationRequestHandler
        handler_class = AutomationRequestHandler
    else:
        handler_class = SimpleHTTPRequestHandler

    server = HTTPServer(("127.0.0.1", port), handler_class)
    t = threading.Thread(target=server.serve_forever, daemon=True)
    t.start()
    return server


class MainWindow(QMainWindow):
    def __init__(self, port: int):
        super().__init__()
        self.setWindowTitle("YouTube Summarizer")
        self.resize(1200, 750)
        self.setMinimumSize(900, 500)

        central = QWidget()
        self.setCentralWidget(central)
        layout = QVBoxLayout(central)
        layout.setContentsMargins(0, 0, 0, 0)

        self.webview = QWebEngineView()

        settings = self.webview.page().settings()
        settings.setAttribute(QWebEngineSettings.WebAttribute.LocalContentCanAccessRemoteUrls, True)
        settings.setAttribute(QWebEngineSettings.WebAttribute.PlaybackRequiresUserGesture, False)
        settings.setAttribute(QWebEngineSettings.WebAttribute.FullScreenSupportEnabled, True)

        layout.addWidget(self.webview)

        self.channel = QWebChannel()
        self.bridge = Bridge()
        self.channel.registerObject("bridge", self.bridge)

        self.webview.page().setWebChannel(self.channel)
        self.webview.setUrl(QUrl(f"http://127.0.0.1:{port}/index.html"))

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

    automation = "--automation" in sys.argv

    port = _find_free_port()
    _start_http_server(port, automation=automation)
    print(f"Server laeuft auf http://127.0.0.1:{port}")

    app = QApplication(sys.argv)
    app.setApplicationName("YouTube Summarizer")
    app.setOrganizationName("youtube-summarizer")

    window = MainWindow(port)

    if automation:
        from app.automation import AutomationRequestHandler
        AutomationRequestHandler.bridge = window.bridge
        AutomationRequestHandler.window = window
        window.bridge.set_window(window)
        print(f"AUTOMATION_URL=http://127.0.0.1:{port}/api")

    window.show()

    sys.exit(app.exec())


if __name__ == "__main__":
    main()
