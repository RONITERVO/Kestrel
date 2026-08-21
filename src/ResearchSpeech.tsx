import { Pause, Play, SkipBack, SkipForward, Square, Volume2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { getLocalSpeechSnapshot, prepareLocalSpeech } from "./api";
import { SpeechLiveCaption } from "./LocalSpeechControls";
import { buildResearchSpeechPassages, type ResearchSpeechScope } from "./researchSpeechContent";
import type { LocalSpeechSnapshot, ResearchReport } from "./types";
import { usePipelinedSpeechPlayer } from "./usePipelinedSpeechPlayer";

const MODEL_KEY = "kestrel.researchSpeech.comfyModel";
const RATE_KEY = "kestrel.researchSpeech.rate";
const SCOPE_KEY = "kestrel.researchSpeech.scope";

function readPreference(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function savePreference(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Playback remains available when WebView preference storage is unavailable.
  }
}

function initialRate(): number {
  const saved = Number(readPreference(RATE_KEY));
  return Number.isFinite(saved) && saved >= 0.8 && saved <= 1.5 ? saved : 1;
}

function initialScope(): ResearchSpeechScope {
  const saved = readPreference(SCOPE_KEY);
  return saved === "summary" || saved === "all" ? saved : "article";
}

interface ResearchSpeechPlayerProps {
  report: ResearchReport;
  onPassageChange: (anchorId: string | null) => void;
}

export function ResearchSpeechPlayer({ report, onPassageChange }: ResearchSpeechPlayerProps) {
  const [snapshot, setSnapshot] = useState<LocalSpeechSnapshot | null>(null);
  const [modelId, setModelId] = useState(() => readPreference(MODEL_KEY) ?? "");
  const [rate, setRate] = useState(initialRate);
  const [scope, setScope] = useState<ResearchSpeechScope>(initialScope);

  const passages = useMemo(() => buildResearchSpeechPassages(report, scope), [report, scope]);
  const selectedModel = snapshot?.voices.find((model) => model.id === modelId) ?? snapshot?.voices[0] ?? null;
  const alignmentModel = snapshot?.transcriptionAvailable ? snapshot.transcribers[0] : undefined;

  const player = usePipelinedSpeechPlayer({
    sourceKind: "research",
    sourceId: report.id,
    passages,
    selectedVoiceModel: selectedModel,
    alignmentModel,
    playbackRate: rate,
    initialDetail: "Checking local ComfyUI voice models...",
    onPassageChange: (passage) => onPassageChange(passage?.anchorId ?? null),
  });

  useEffect(() => {
    let active = true;
    void getLocalSpeechSnapshot()
      .then((next) => {
        if (!active) return;
        setSnapshot(next);
        player.setDetail(next.detail);
        const selected = next.voices.find((model) => model.id === modelId) ?? next.voices[0];
        setModelId(selected?.id ?? "");
        if (selected) savePreference(MODEL_KEY, selected.id);
        if (next.narrationAvailable && !next.comfyReady) {
          player.setDetail("Starting the private ComfyUI voice engine in the background...");
          void prepareLocalSpeech().then((ready) => {
            if (!active) return;
            setSnapshot(ready);
            player.setDetail(ready.detail);
          }).catch((cause) => {
            if (!active) return;
            player.setDetail(`ComfyUI will retry when Play is pressed: ${String(cause)}`);
          });
        }
      })
      .catch((cause) => {
        if (!active) return;
        player.setError(String(cause));
        player.setStatus("error");
      });
    return () => {
      active = false;
    };
  }, []);

  const chooseModel = (nextModelId: string) => {
    player.clearModelCache();
    setModelId(nextModelId);
    savePreference(MODEL_KEY, nextModelId);
  };

  const chooseRate = (nextRate: number) => {
    if (!Number.isFinite(nextRate) || nextRate < 0.8 || nextRate > 1.5) return;
    setRate(nextRate);
    savePreference(RATE_KEY, String(nextRate));
  };

  const chooseScope = (nextScope: ResearchSpeechScope) => {
    setScope(nextScope);
    savePreference(SCOPE_KEY, nextScope);
  };

  const unavailable = !snapshot?.narrationAvailable || !selectedModel;
  const activePassage = player.currentPassage;
  const statusText = !snapshot
    ? "Checking local ComfyUI"
    : unavailable
      ? "ComfyUI TTS unavailable"
      : player.status === "preparing"
        ? `Preparing ${activePassage?.label ?? "passage"} - ${player.elapsed}s`
        : player.status === "playing"
          ? `Reading ${activePassage?.label ?? "report"}`
          : player.status === "paused"
            ? `Paused at ${activePassage?.label ?? "report"}`
            : player.status === "complete"
              ? "Finished"
              : "Ready with local ComfyUI";

  return (
    <section className="research-speech-player" aria-label="Listen to report">
      <audio ref={player.audioRef} {...player.audioProps} />
      <div className="speech-player-heading">
        <span className="speech-player-icon"><Volume2 size={17} /></span>
        <span><strong>Listen</strong><small>{statusText}</small></span>
      </div>
      <div className="speech-transport" aria-label="Speech controls">
        <button
          type="button"
          aria-label="Previous passage"
          title="Previous passage"
          disabled={unavailable || player.status === "preparing" || player.currentIndex === 0}
          onClick={() => player.navigateTo(player.currentIndex - 1)}
        >
          <SkipBack size={15} />
        </button>
        <button
          type="button"
          className="speech-play"
          aria-label={player.status === "playing" ? "Pause report" : "Play report"}
          disabled={unavailable || player.status === "preparing"}
          onClick={player.togglePlayback}
        >
          {player.status === "playing" ? <Pause size={16} /> : <Play size={16} />}
        </button>
        <button
          type="button"
          aria-label="Stop report"
          disabled={unavailable || !["preparing", "playing", "paused"].includes(player.status)}
          onClick={player.stopPlayback}
        >
          <Square size={13} />
        </button>
        <button
          type="button"
          aria-label="Next passage"
          title="Next passage"
          disabled={unavailable || player.status === "preparing" || player.currentIndex >= passages.length - 1}
          onClick={() => player.navigateTo(player.currentIndex + 1)}
        >
          <SkipForward size={15} />
        </button>
      </div>
      <div className="speech-options">
        <label>Read
          <select
            aria-label="Reading length"
            value={scope}
            disabled={player.status === "preparing"}
            onChange={(event) => chooseScope(event.target.value as ResearchSpeechScope)}
          >
            <option value="summary">Summary</option>
            <option value="article">Article</option>
            <option value="all">Article + sources</option>
          </select>
        </label>
        <label>ComfyUI voice
          <select
            aria-label="ComfyUI voice model"
            value={selectedModel?.id ?? ""}
            disabled={unavailable || ["preparing", "playing", "paused"].includes(player.status)}
            onChange={(event) => chooseModel(event.target.value)}
          >
            {!snapshot?.voices.length && <option value="">No local TTS model</option>}
            {snapshot?.voices.map((model) => <option value={model.id} key={model.id}>{model.name}</option>)}
          </select>
        </label>
        <label>Playback speed
          <select
            aria-label="Playback speed"
            value={rate}
            disabled={unavailable}
            onChange={(event) => chooseRate(Number(event.currentTarget.value))}
          >
            <option value={0.8}>0.8x</option>
            <option value={1}>1x</option>
            <option value={1.15}>1.15x</option>
            <option value={1.3}>1.3x</option>
            <option value={1.5}>1.5x</option>
          </select>
        </label>
      </div>
      <div
        className="speech-progress"
        role="progressbar"
        aria-label="Report speech progress"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(player.progress)}
      >
        <span style={{ width: `${player.progress}%` }} />
      </div>
      {["playing", "paused"].includes(player.status) && activePassage && (
        <div className="speech-current-passage">
          <input
            type="range"
            aria-label="Current passage position"
            min={0}
            max={Math.max(player.speechDuration, 0.01)}
            step={0.01}
            value={Math.min(player.speechSeconds, Math.max(player.speechDuration, 0.01))}
            onChange={(event) => player.seekSpeech(event.currentTarget.valueAsNumber)}
          />
          <SpeechLiveCaption
            text={activePassage.text}
            seconds={player.speechSeconds}
            duration={player.speechDuration}
            timings={player.speechTimings}
            onSeek={player.seekSpeech}
          />
        </div>
      )}
      <div className="speech-player-meta">
        <span>{player.error ?? player.detail}</span>
        {!unavailable && (
          <span>
            {player.bufferingIndex !== null
              ? `Buffering ${player.bufferingIndex + 1}`
              : `${Math.min(player.currentIndex + 1, passages.length)} / ${passages.length}`}
          </span>
        )}
      </div>
      {player.error && <p className="speech-error" role="alert">ComfyUI narration stopped: {player.error}</p>}
    </section>
  );
}
