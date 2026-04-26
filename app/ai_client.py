import httpx
from app.config import config

DEFAULT_SYSTEM_PROMPT = """You are a helpful assistant that summarizes YouTube video transcripts.
Provide a clear, structured summary in the same language as the transcript.
Include:
- A short overview (1-2 sentences)
- Key points as bullet points
- Main conclusions or takeaways

Format your response as Markdown."""


async def summarize(transcript: str, system_prompt: str | None = None) -> str:
    ai = config.ai
    headers = {"Content-Type": "application/json"}
    if ai.api_key:
        headers["Authorization"] = f"Bearer {ai.api_key}"

    payload = {
        "model": ai.model,
        "messages": [
            {"role": "system", "content": system_prompt or DEFAULT_SYSTEM_PROMPT},
            {"role": "user", "content": f"Please summarize the following YouTube video transcript:\n\n{transcript}"},
        ],
        "temperature": 0.5,
        "max_tokens": 8192,
    }

    async with httpx.AsyncClient(timeout=120) as client:
        response = await client.post(ai.endpoint, headers=headers, json=payload)
        response.raise_for_status()
        data = response.json()
        return data["choices"][0]["message"]["content"]
