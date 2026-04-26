from pathlib import Path
from sqlalchemy import create_engine, text
from sqlalchemy.orm import sessionmaker, Session

DB_PATH = Path(__file__).resolve().parent.parent / "videos.db"
ENGINE = create_engine(f"sqlite:///{DB_PATH}", echo=False)
SessionLocal = sessionmaker(bind=ENGINE)


def get_session() -> Session:
    return SessionLocal()


def init_db():
    from app.models import Base
    Base.metadata.create_all(bind=ENGINE)
    _migrate()


def _migrate():
    with ENGINE.connect() as conn:
        result = conn.execute(text("PRAGMA table_info(videos)"))
        columns = {row[1] for row in result.fetchall()}
        if "thumbnail_data" not in columns:
            conn.execute(text("ALTER TABLE videos ADD COLUMN thumbnail_data BLOB"))
            conn.commit()
