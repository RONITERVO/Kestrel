"""Owned, offline CPU speech worker. stdin/stdout carry bounded JSON lines, never commands.

The language model remains in RuntimeManager. No HTTP listener, runtime downloads, CUDA,
shell execution, or access to task tools exists here. Setup prepares every dependency/asset.
"""
import base64
import concurrent.futures
import gc
import json
import os
from pathlib import Path
import socket
import sys
import threading

ROOT = Path(sys.argv[1]).resolve()
os.environ.update(HF_HUB_OFFLINE="1", TRANSFORMERS_OFFLINE="1", HF_HUB_DISABLE_TELEMETRY="1",
                  CUDA_VISIBLE_DEVICES="", OMP_NUM_THREADS="2", MKL_NUM_THREADS="2")


def no_network(*args, **kwargs):
    raise RuntimeError("Conversation speech is offline. Repair missing assets in Setup.")


socket.create_connection = no_network
socket.socket.connect = no_network
socket.socket.connect_ex = no_network
# Library diagnostics must never corrupt the protocol.
PROTOCOL = sys.stdout
PROTOCOL.reconfigure(encoding="utf-8", errors="strict")
sys.stderr.reconfigure(encoding="utf-8", errors="replace")
sys.stdout = sys.stderr
import numpy as np
import torch
from faster_whisper import WhisperModel
from pocket_tts import TTSModel

torch.set_num_threads(2)
torch.set_num_interop_threads(1)
LANGUAGES = {"en": "english", "fr": "french", "de": "german", "pt": "portuguese",
             "it": "italian", "es": "spanish", "nl": "dutch"}
MODELS = {}
VOICES = {}
STT = None
OUTPUT_LOCK = threading.Lock()
JOBS_LOCK = threading.Lock()
JOBS = {}
TTS_POOL = concurrent.futures.ThreadPoolExecutor(max_workers=1, thread_name_prefix="pocket")
STT_POOL = concurrent.futures.ThreadPoolExecutor(max_workers=1, thread_name_prefix="whisper")


def emit(job, kind, **values):
    with OUTPUT_LOCK:
        PROTOCOL.write(json.dumps({"id": job, "kind": kind, **values}, ensure_ascii=False) + "\n")
        PROTOCOL.flush()


def tts(language):
    if language not in LANGUAGES:
        raise ValueError("Unsupported conversation language")
    if language not in MODELS:
        # All TTS work shares one executor. Drop the previous model and its conditioning
        # before loading the user's new selection; never retain a language-model cache.
        VOICES.clear()
        MODELS.clear()
        gc.collect()
        MODELS[language] = TTSModel.load_model(config=ROOT / "models" / language / "config.yaml")
        MODELS[language].has_voice_cloning = (ROOT / "models" / language / "voice-cloning.safetensors").is_file()
    return MODELS[language]


def stt():
    global STT
    if STT is None:
        STT = WhisperModel(str(ROOT / "models" / "whisper-base"), device="cpu",
                           compute_type="int8", cpu_threads=2, num_workers=1, local_files_only=True)
    return STT


def voice(model, language, request):
    if request.get("voicePath") and not model.has_voice_cloning:
        raise ValueError("This Pocket language has supplied voices only. Import authorized voice-cloning weights to use a recorded voice; your existing manual TTS voice remains available.")
    source = request.get("voicePath") or str(ROOT / "models" / language / "voice.safetensors")
    path = Path(source)
    if not path.is_absolute() or not path.is_file():
        raise ValueError("The selected local voice source is missing")
    key = (language, request.get("voiceFingerprint", "default"))
    if key not in VOICES:
        # Voice conditioning is model-specific and cleared whenever the language changes.
        if len(VOICES) >= 14:
            VOICES.pop(next(iter(VOICES)))
        VOICES[key] = model.get_state_for_audio_prompt(path, truncate=True)
    return VOICES[key]


def run(request, cancel):
    job = request["id"]
    try:
        if cancel.is_set():
            return
        operation = request["op"]
        if operation == "status":
            emit(job, "status", pid=os.getpid(), models={code: id(model) for code, model in MODELS.items()},
                 stt=id(STT) if STT is not None else None, cuda=torch.version.cuda,
                 devices={code: str(model.device) for code, model in MODELS.items()})
        elif operation == "warm_tts":
            language = request["language"]
            model = tts(language)
            voice(model, language, {})
        elif operation == "warm_stt":
            stt()
        elif operation == "speak":
            language = request["language"]
            if not MODELS:
                tts(language)  # An explicit new utterance may recover a crashed worker.
            if language not in MODELS:
                raise ValueError("Conversation language changed. Prepare the selected language before speaking again.")
            model = MODELS[language]
            state = voice(model, language, request)
            for chunk in model.generate_audio_stream(state, request["text"], stop=cancel):
                if cancel.is_set():
                    break
                pcm = (chunk.detach().cpu().numpy().clip(-1, 1) * 32767).astype("<i2")
                # Short bounded protocol frames also bound cancellation latency in the reader.
                for start in range(0, pcm.size, 2400):
                    if cancel.is_set():
                        break
                    emit(job, "audio", sampleRate=model.sample_rate,
                         pcm=base64.b64encode(pcm[start:start + 2400].tobytes()).decode("ascii"))
        elif operation == "transcribe":
            pcm = base64.b64decode(request["pcm"], validate=True)
            if not 3200 <= len(pcm) <= 960000 or len(pcm) % 2:
                raise ValueError("Speech must contain 0.1–30 seconds of mono 16 kHz PCM")
            audio = np.frombuffer(pcm, dtype="<i2").astype(np.float32) / 32768
            language = request["language"]
            if language != "auto" and language not in LANGUAGES:
                raise ValueError("Unsupported transcription language")
            segments, info = stt().transcribe(audio, language=None if language == "auto" else language,
                beam_size=1, word_timestamps=True, condition_on_previous_text=False,
                vad_filter=True, vad_parameters={"min_silence_duration_ms": 300})
            words, text = [], []
            for segment in segments:
                if cancel.is_set():
                    break
                # Whisper can hallucinate on silence; retain only credible speech segments.
                if segment.no_speech_prob > 0.6 or segment.avg_logprob < -1.0:
                    continue
                text.append(segment.text)
                words.extend({"value": w.word.strip(), "start": w.start, "end": w.end}
                             for w in segment.words or [] if w.word.strip())
            if not cancel.is_set():
                emit(job, "transcript", text="".join(text).strip(), language=info.language, words=words)
        else:
            raise ValueError("Unknown speech operation")
    except Exception as error:
        emit(job, "error", detail=str(error)[:1500])
    finally:
        emit(job, "cancelled" if cancel.is_set() else "done")
        with JOBS_LOCK:
            JOBS.pop(job, None)


def main():
    while True:
        line = sys.stdin.buffer.readline(1_400_001)
        if not line:
            break
        if len(line) > 1_400_000 or not line.endswith(b"\n"):
            raise ValueError("Oversized speech request")
        request = json.loads(line)
        job = request["id"]
        if request["op"] == "cancel":
            with JOBS_LOCK:
                event = JOBS.get(job)
                if event:
                    event.set()
            continue
        with JOBS_LOCK:
            if job in JOBS or len(JOBS) >= 4:
                emit(job, "error", detail="Speech queue is full; retry after the current utterance")
                continue
            cancel = threading.Event()
            JOBS[job] = cancel
        pool = STT_POOL if request["op"] in ("transcribe", "warm_stt") else TTS_POOL
        pool.submit(run, request, cancel)
    with JOBS_LOCK:
        for event in JOBS.values():
            event.set()
    TTS_POOL.shutdown(wait=True, cancel_futures=True)
    STT_POOL.shutdown(wait=True, cancel_futures=True)


if __name__ == "__main__":
    main()
