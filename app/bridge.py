import json
import asyncio
from PySide6.QtCore import QObject, Signal, Slot, Property

from app.config import config
from app.database import get_session, init_db
from app.models import Video
from app.youtube import extract_video_id, get_video_url, fetch_video_info, fetch_transcript, download_thumbnail, fetch_chapters, transcript_to_text
from app.ai_client import summarize


class Bridge(QObject):
    videos_loaded = Signal(str)
    video_detail_loaded = Signal(str)
    transcript_loaded = Signal(int, str)
    chapters_loaded = Signal(int, str)
    summary_loaded = Signal(int, str)
    video_added = Signal(str)
    video_deleted = Signal(int)
    error = Signal(str)
    config_loaded = Signal(str)
    status_update = Signal(str)

    def __init__(self, parent=None):
        super().__init__(parent)
        init_db()

    @Slot(str)
    def add_video(self, url: str):
        video_id = extract_video_id(url)
        if not video_id:
            self.error.emit("Ungültige YouTube-URL oder Video-ID")
            return

        session = get_session()
        try:
            existing = session.query(Video).filter_by(video_id=video_id).first()
            if existing:
                self.error.emit("Video bereits in der Liste vorhanden")
                return

            info = fetch_video_info(video_id)
            title = info["title"]
            thumb_url = info["thumbnail_url"]
            thumb_data = download_thumbnail(video_id)

            video = Video(
                video_id=video_id,
                url=get_video_url(video_id),
                title=title,
                thumbnail_url=thumb_url,
                thumbnail_data=thumb_data,
            )
            session.add(video)
            session.commit()
            video_data = video.to_dict()
            self.video_added.emit(json.dumps(video_data, ensure_ascii=False))
            self.status_update.emit(f"Video hinzugefügt: {title}")

            self._fetch_transcript_async(video.id, video_id)
        except Exception as e:
            self.error.emit(f"Fehler beim Hinzufügen: {str(e)}")
        finally:
            session.close()

    def _fetch_transcript_async(self, db_id: int, video_id: str):
        def _run():
            loop = asyncio.new_event_loop()
            asyncio.set_event_loop(loop)
            try:
                transcript = fetch_transcript(video_id)
                chapters = fetch_chapters(video_id)
                self.transcript_loaded.emit(db_id, transcript)
                if chapters:
                    self.chapters_loaded.emit(db_id, chapters)
                session = get_session()
                try:
                    video = session.query(Video).filter_by(id=db_id).first()
                    if video:
                        video.transcript = transcript
                        video.chapters = chapters
                        session.commit()
                finally:
                    session.close()
                self.status_update.emit("Transkript geladen")
            except Exception as e:
                self.error.emit(f"Transkript-Fehler: {str(e)}")
            finally:
                loop.close()

        import threading
        t = threading.Thread(target=_run, daemon=True)
        t.start()

    @Slot(int)
    def get_video_detail(self, video_id: int):
        session = get_session()
        try:
            video = session.query(Video).filter_by(id=video_id).first()
            if not video:
                self.error.emit("Video nicht gefunden")
                return
            self.video_detail_loaded.emit(json.dumps(video.to_dict(), ensure_ascii=False))
        finally:
            session.close()

    @Slot()
    def get_videos(self):
        session = get_session()
        try:
            videos = session.query(Video).order_by(Video.created_at.desc()).all()
            data = [v.to_dict() for v in videos]
            self.videos_loaded.emit(json.dumps(data, ensure_ascii=False))
        finally:
            session.close()

    @Slot(int, str)
    def summarize_video(self, db_id: int, system_prompt: str = ""):
        session = get_session()
        try:
            video = session.query(Video).filter_by(id=db_id).first()
            if not video:
                self.error.emit("Video nicht gefunden")
                return
            if not video.transcript:
                self.error.emit("Kein Transkript vorhanden – bitte erst Transkript laden")
                return
            transcript_text = transcript_to_text(video.transcript)
            session.close()
        except Exception:
            session.close()
            return

        def _run():
            loop = asyncio.new_event_loop()
            asyncio.set_event_loop(loop)
            try:
                self.status_update.emit("Zusammenfassung wird erstellt...")
                result = loop.run_until_complete(summarize(transcript_text, system_prompt or None))
                self.summary_loaded.emit(db_id, result)
                session2 = get_session()
                try:
                    v = session2.query(Video).filter_by(id=db_id).first()
                    if v:
                        v.summary = result
                        session2.commit()
                finally:
                    session2.close()
                self.status_update.emit("Zusammenfassung fertig")
            except Exception as e:
                error_msg = str(e)
                if "401" in error_msg or "403" in error_msg:
                    error_msg = "API-Key ungültig – bitte in den Einstellungen prüfen"
                self.error.emit(f"KI-Fehler: {error_msg}")
            finally:
                loop.close()

        import threading
        t = threading.Thread(target=_run, daemon=True)
        t.start()

    @Slot(int)
    def delete_video(self, db_id: int):
        session = get_session()
        try:
            video = session.query(Video).filter_by(id=db_id).first()
            if video:
                session.delete(video)
                session.commit()
                self.video_deleted.emit(db_id)
                self.status_update.emit("Video gelöscht")
        except Exception as e:
            self.error.emit(f"Löschfehler: {str(e)}")
        finally:
            session.close()

    @Slot()
    def get_config(self):
        data = config.ai.to_dict()
        self.config_loaded.emit(json.dumps(data, ensure_ascii=False))

    @Slot(str, str, str)
    def save_config(self, provider: str, api_key: str, model: str):
        config.ai.provider = provider
        config.ai.api_key = api_key
        config.ai.model = model
        config.save()
        self.config_loaded.emit(json.dumps(config.ai.to_dict(), ensure_ascii=False))
        self.status_update.emit("Konfiguration gespeichert")
