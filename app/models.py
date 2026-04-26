import base64
from datetime import datetime, timezone
from sqlalchemy import Column, Integer, String, Text, DateTime, LargeBinary
from sqlalchemy.orm import DeclarativeBase


class Base(DeclarativeBase):
    pass


class Video(Base):
    __tablename__ = "videos"

    id = Column(Integer, primary_key=True, autoincrement=True)
    video_id = Column(String(32), unique=True, nullable=False)
    url = Column(String(256), nullable=False)
    title = Column(String(512), nullable=False)
    thumbnail_url = Column(String(512), nullable=False)
    thumbnail_data = Column(LargeBinary, nullable=True)
    transcript = Column(Text, nullable=True)
    summary = Column(Text, nullable=True)
    created_at = Column(DateTime, default=lambda: datetime.now(timezone.utc))
    updated_at = Column(DateTime, default=lambda: datetime.now(timezone.utc), onupdate=lambda: datetime.now(timezone.utc))

    def to_dict(self) -> dict:
        thumb = None
        if self.thumbnail_data:
            thumb = "data:image/jpeg;base64," + base64.b64encode(self.thumbnail_data).decode("ascii")
        return {
            "id": self.id,
            "video_id": self.video_id,
            "url": self.url,
            "title": self.title,
            "thumbnail": thumb,
            "thumbnail_url": self.thumbnail_url,
            "transcript": self.transcript,
            "summary": self.summary,
            "created_at": self.created_at.isoformat() if self.created_at else None,
            "updated_at": self.updated_at.isoformat() if self.updated_at else None,
        }
