import json
from http.server import SimpleHTTPRequestHandler
from urllib.parse import urlparse, parse_qs


class AutomationRequestHandler(SimpleHTTPRequestHandler):
    bridge = None
    window = None

    def log_message(self, format, *args):
        pass

    def _send_json(self, data, status=200):
        body = json.dumps(data, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(body)

    def _send_error(self, message, status=400):
        self._send_json({"error": message}, status)

    def _read_body(self):
        length = int(self.headers.get("Content-Length", 0))
        if length:
            return json.loads(self.rfile.read(length))
        return {}

    def _parse_path(self):
        parsed = urlparse(self.path)
        path = parsed.path.rstrip("/") or "/"
        params = parse_qs(parsed.query)
        return path, params

    def _path_id(self, prefix):
        _, _, tail = self._parse_path()[0].partition(prefix)
        return tail.strip("/")

    # --- HTTP methods ---

    def do_GET(self):
        path, _ = self._parse_path()

        if path == "/api/health":
            if self.bridge:
                self._send_json({"status": "ok"})
            else:
                self._send_json({"status": "initializing"}, 503)

        elif path == "/api/videos":
            self._get_videos()

        elif path.startswith("/api/video/"):
            video_id = self._path_id("/api/video/")
            self._get_video(video_id)

        elif path == "/api/screenshot":
            self._screenshot()

        elif path == "/api/status":
            self._status()

        else:
            super().do_GET()

    def do_POST(self):
        path, _ = self._parse_path()

        if path == "/api/add-video":
            self._add_video(self._read_body().get("url", ""))

        elif path.startswith("/api/summarize/"):
            video_id = self._path_id("/api/summarize/")
            prompt = self._read_body().get("system_prompt", "")
            self._summarize(video_id, prompt)

        else:
            self._send_error("Not found", 404)

    def do_DELETE(self):
        path, _ = self._parse_path()

        if path.startswith("/api/video/"):
            self._delete_video(self._path_id("/api/video/"))
        else:
            self._send_error("Not found", 404)

    def do_OPTIONS(self):
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, DELETE, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")
        self.end_headers()

    # --- API implementations ---

    def _get_videos(self):
        from app.database import get_session
        from app.models import Video
        session = get_session()
        try:
            videos = session.query(Video).order_by(Video.created_at.desc()).all()
            self._send_json({"videos": [v.to_dict() for v in videos]})
        finally:
            session.close()

    def _get_video(self, video_id_str):
        vid = self._parse_int(video_id_str)
        if vid is None:
            self._send_error("Invalid video ID", 400)
            return

        from app.database import get_session
        from app.models import Video
        session = get_session()
        try:
            video = session.query(Video).filter_by(id=vid).first()
            if video:
                self._send_json(video.to_dict())
            else:
                self._send_error("Video not found", 404)
        finally:
            session.close()

    def _add_video(self, url):
        if not url:
            self._send_error("Missing 'url' field", 400)
            return
        if not self.bridge:
            self._send_error("Bridge not ready", 503)
            return
        try:
            self.bridge.add_video(url)
            self._send_json({"status": "ok", "message": "Video wird hinzugefuegt"})
        except Exception as e:
            self._send_error(str(e), 500)

    def _delete_video(self, video_id_str):
        vid = self._parse_int(video_id_str)
        if vid is None:
            self._send_error("Invalid video ID", 400)
            return
        if not self.bridge:
            self._send_error("Bridge not ready", 503)
            return
        try:
            self.bridge.delete_video(vid)
            self._send_json({"status": "ok"})
        except Exception as e:
            self._send_error(str(e), 500)

    def _summarize(self, video_id_str, system_prompt):
        vid = self._parse_int(video_id_str)
        if vid is None:
            self._send_error("Invalid video ID", 400)
            return
        if not self.bridge:
            self._send_error("Bridge not ready", 503)
            return
        try:
            self.bridge.summarize_video(vid, system_prompt)
            self._send_json({"status": "ok", "message": "Zusammenfassung wird erstellt"})
        except Exception as e:
            self._send_error(str(e), 500)

    def _screenshot(self):
        if not self.bridge:
            self._send_error("Bridge not ready", 503)
            return
        png = self.bridge.request_screenshot()
        if png:
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(png)))
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            self.wfile.write(png)
        else:
            self._send_error("Screenshot failed", 500)

    def _status(self):
        if not self.bridge:
            self._send_error("Bridge not ready", 503)
            return
        self._send_json(self.bridge.get_status())

    @staticmethod
    def _parse_int(s):
        try:
            return int(s)
        except (ValueError, TypeError):
            return None
