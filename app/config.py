import json
import os
from pathlib import Path
from typing import Optional

_CONFIG_PATH = Path(__file__).resolve().parent.parent / "config.json"

DEFAULT_ENDPOINTS = {
    "opencode_zen": "https://opencode.ai/zen/v1/chat/completions",
    "opencode_go": "https://opencode.ai/zen/go/v1/chat/completions",
    "openrouter": "https://openrouter.ai/api/v1/chat/completions",
    "ollama": "http://localhost:11434/v1/chat/completions",
}


class AIConfig:
    def __init__(self, data: dict):
        self.provider: str = data.get("provider", "opencode_go")
        self.api_key: str = data.get("api_key", os.environ.get("AI_API_KEY", ""))
        self.model: str = data.get("model", "qwen3.5-plus")
        self.endpoint_override: Optional[str] = data.get("endpoint_override")

    @property
    def endpoint(self) -> str:
        if self.endpoint_override:
            return self.endpoint_override
        return DEFAULT_ENDPOINTS.get(self.provider, self.endpoint_override or "")

    def to_dict(self) -> dict:
        return {
            "provider": self.provider,
            "api_key": self.api_key,
            "model": self.model,
            "endpoint_override": self.endpoint_override,
        }


class Config:
    def __init__(self, path: Optional[Path] = None):
        self.path = path or _CONFIG_PATH
        self.ai = AIConfig({})
        self.load()

    def load(self):
        if self.path.exists():
            with open(self.path, "r", encoding="utf-8") as f:
                data = json.load(f)
        else:
            data = {}
        self.ai = AIConfig(data.get("ai", {}))

    def save(self):
        data = {"ai": self.ai.to_dict()}
        with open(self.path, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2, ensure_ascii=False)


config = Config()
