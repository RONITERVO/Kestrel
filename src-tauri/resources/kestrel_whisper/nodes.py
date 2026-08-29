"""Small, auditable ComfyUI boundary for Kestrel's local speech contract.

The setup flow downloads the selected OpenAI Whisper checkpoint before offline use. This adapter
never asks Whisper to download a model and exposes bounded transcript, timing, and context metadata.
"""

import gc
import hashlib
import json
import math
import os
import threading

import folder_paths
import numpy as np
import torch
import torchaudio
import whisper


MODEL_ROOT = os.path.join(folder_paths.models_dir, "stt", "whisper")
EXPECTED_MODELS = {
    "large-v3-turbo": "aff26ae408abcba5fbf8813c21e62b0941638c5f6eebfb145be0c9839262a19a",
}
_MODEL = None
_MODEL_NAME = None
_MODEL_LOCK = threading.Lock()
MUSIC_CONTEXT_MODE = "music-repeat"
MUSIC_CONTEXT_SEAM_SECONDS = 1.0
MAX_MUSIC_CONTEXT_SOURCE_SECONDS = 330.0


def _available_models():
    if not os.path.isdir(MODEL_ROOT):
        return ["large-v3-turbo"]
    names = sorted(
        os.path.splitext(name)[0]
        for name in os.listdir(MODEL_ROOT)
        if name.endswith(".pt")
        and os.path.splitext(name)[0] in EXPECTED_MODELS
        and os.path.isfile(os.path.join(MODEL_ROOT, name))
    )
    return names or ["large-v3-turbo"]


def _release_model():
    global _MODEL, _MODEL_NAME
    with _MODEL_LOCK:
        model = _MODEL
        _MODEL = None
        _MODEL_NAME = None
        if model is not None:
            del model
    gc.collect()
    if torch.cuda.is_available():
        torch.cuda.empty_cache()


def _load_model(name):
    global _MODEL, _MODEL_NAME
    model_path = os.path.join(MODEL_ROOT, f"{name}.pt")
    if not os.path.isfile(model_path):
        raise RuntimeError(
            f"Whisper checkpoint is missing: {model_path}. Open Kestrel Setup and resume Local voice and dictation."
        )
    with _MODEL_LOCK:
        if _MODEL is None or _MODEL_NAME != name:
            digest = hashlib.sha256()
            with open(model_path, "rb") as checkpoint:
                for chunk in iter(lambda: checkpoint.read(4 * 1024 * 1024), b""):
                    digest.update(chunk)
            if digest.hexdigest().lower() != EXPECTED_MODELS.get(name, ""):
                raise RuntimeError(
                    "Whisper checkpoint failed its offline integrity check. Open Kestrel Setup and resume Local voice and dictation."
                )
            old_model = _MODEL
            _MODEL = None
            _MODEL_NAME = None
            if old_model is not None:
                del old_model
                gc.collect()
                if torch.cuda.is_available():
                    torch.cuda.empty_cache()
            device = "cuda" if torch.cuda.is_available() else "cpu"
            # download_root is fixed to the already verified local folder. Kestrel launches ComfyUI
            # with Hugging Face and Transformers offline flags, and setup verifies this checkpoint.
            _MODEL = whisper.load_model(name, device=device, download_root=MODEL_ROOT)
            _MODEL_NAME = name
        return _MODEL


def _finite(value, fallback=0.0):
    try:
        number = float(value)
    except (TypeError, ValueError):
        return fallback
    return number if math.isfinite(number) and number >= 0.0 else fallback


def _audio_mono_16khz(audio):
    waveform = audio["waveform"]
    sample_rate = int(audio["sample_rate"])
    if waveform.ndim != 3 or waveform.shape[0] < 1 or waveform.shape[-1] < 1:
        raise ValueError("Kestrel Whisper received an empty or invalid ComfyUI audio tensor")
    mono = waveform[0].detach().float().cpu().mean(dim=0)
    if sample_rate != 16000:
        mono = torchaudio.functional.resample(mono, sample_rate, 16000)
    return np.ascontiguousarray(mono.numpy(), dtype=np.float32)


class KestrelWhisper:
    @classmethod
    def INPUT_TYPES(cls):
        return {
            "required": {
                "audio": ("AUDIO",),
                "model": (_available_models(),),
                "language": ("STRING", {"default": "auto", "multiline": False}),
                "prompt": ("STRING", {"default": "", "multiline": True}),
                "context_mode": (["single", MUSIC_CONTEXT_MODE], {"default": "single"}),
            }
        }

    RETURN_TYPES = ("STRING", "STRING", "STRING", "STRING")
    RETURN_NAMES = ("transcript", "segments_json", "words_json", "context_json")
    FUNCTION = "transcribe"
    CATEGORY = "Kestrel/Audio"

    def transcribe(self, audio, model, language="auto", prompt="", context_mode="single"):
        selected = str(model).strip()
        if selected not in _available_models():
            raise ValueError("Kestrel Whisper model selection is not installed")
        mode = str(context_mode).strip()
        if mode not in ("single", MUSIC_CONTEXT_MODE):
            raise ValueError("Kestrel Whisper received an unsupported context mode")
        source_audio = _audio_mono_16khz(audio)
        source_duration = float(source_audio.shape[0]) / 16000.0
        second_start = 0.0
        transcription_audio = source_audio
        if mode == MUSIC_CONTEXT_MODE:
            if source_duration > MAX_MUSIC_CONTEXT_SOURCE_SECONDS:
                raise ValueError(
                    "Kestrel's repeated music context is limited to 330 seconds of source audio"
                )
            seam = np.zeros(int(MUSIC_CONTEXT_SEAM_SECONDS * 16000), dtype=np.float32)
            # The first copy lets Whisper learn the singer, vocabulary, and recent lyric style.
            # Silence gives the decoder an unambiguous song boundary before the saved copy.
            transcription_audio = np.concatenate((source_audio, seam, source_audio))
            second_start = source_duration + MUSIC_CONTEXT_SEAM_SECONDS
        prompt_text = str(prompt).strip() or None
        result = _load_model(selected).transcribe(
            transcription_audio,
            language=None if str(language).strip().lower() in ("", "auto") else str(language).strip(),
            initial_prompt=prompt_text,
            condition_on_previous_text=True,
            # Music mode uses only a bounded opening excerpt. Carrying it leaves room for recent
            # decoded lyrics while keeping the song's opening vocabulary available at copy two.
            carry_initial_prompt=mode == MUSIC_CONTEXT_MODE and prompt_text is not None,
            word_timestamps=True,
            fp16=torch.cuda.is_available(),
            verbose=False,
        )
        segments = []
        words = []
        for raw_segment in result.get("segments", []):
            segment = {
                "value": str(raw_segment.get("text", "")).strip(),
                "start": _finite(raw_segment.get("start")),
                "end": _finite(raw_segment.get("end")),
            }
            segment["end"] = max(segment["start"], segment["end"])
            if segment["value"]:
                segments.append(segment)
            for raw_word in raw_segment.get("words") or []:
                word = {
                    "value": str(raw_word.get("word", "")).strip(),
                    "start": _finite(raw_word.get("start"), segment["start"]),
                    "end": _finite(raw_word.get("end"), segment["end"]),
                }
                word["end"] = max(word["start"], word["end"])
                if word["value"]:
                    words.append(word)
        return (
            str(result.get("text", "")).strip(),
            json.dumps(segments, ensure_ascii=False, separators=(",", ":")),
            json.dumps(words, ensure_ascii=False, separators=(",", ":")),
            json.dumps(
                {
                    "mode": mode,
                    "sourceDuration": source_duration,
                    "secondStart": second_start,
                    "secondEnd": second_start + source_duration,
                    "seamSeconds": MUSIC_CONTEXT_SEAM_SECONDS if mode == MUSIC_CONTEXT_MODE else 0.0,
                },
                ensure_ascii=False,
                separators=(",", ":"),
            ),
        )


NODE_CLASS_MAPPINGS = {"KestrelWhisper": KestrelWhisper}
NODE_DISPLAY_NAME_MAPPINGS = {"KestrelWhisper": "Kestrel Whisper (offline)"}


try:
    from aiohttp import web
    from server import PromptServer

    @PromptServer.instance.routes.post("/kestrel/speech/free")
    async def kestrel_speech_free(_request):
        _release_model()
        return web.json_response({"released": True})
except (ImportError, AttributeError, RuntimeError):
    # ComfyUI can import custom nodes in tooling contexts without an active PromptServer.
    pass
