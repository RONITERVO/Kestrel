import { Pause, Play, SkipBack, SkipForward, Square, Volume2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { SpeechLiveCaption, useSpeech } from "../speech/LocalSpeechControls";
import { buildResearchSpeechPassages, type ResearchSpeechScope } from "./researchSpeechContent";
import { type SpeechProgressState } from "../../shared/components/spokenHighlight";
import type { ResearchReport } from "../../contracts/index";
import { usePipelinedSpeechPlayer } from "../speech/usePipelinedSpeechPlayer";

const MODEL_KEY = "kestrel.researchSpeech.comfyModel";
const VOICE_KEY = "kestrel.researchSpeech.voiceProfile";
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
  const saved = readPreference(RATE_KEY);
  const parsed = saved ? Number(saved) : 1;
  return Number.isFinite(parsed) && parsed >= 0.8 && parsed <= 1.5 ? parsed : 1;
}

function initialScope(): ResearchSpeechScope {
  const saved = readPreference(SCOPE_KEY);
  return saved === "summary" || saved === "all" ? saved : "article";
}

interface ResearchSpeechPlayerProps {
  report: ResearchReport;
  onPassageChange?: (anchorId: string | null, passageId?: string | null) => void;
  onSpeechProgress?: (progress: SpeechProgressState | null) => void;
}

export function ResearchSpeechPlayer({ report, onPassageChange, onSpeechProgress }: ResearchSpeechPlayerProps) {
  const { snapshot, refresh, prepare, openVoiceLibrary } = useSpeech();
  const [modelId, setModelId] = useState(() => readPreference(MODEL_KEY) ?? "");
  const [voiceProfileId, setVoiceProfileId] = useState(() => readPreference(VOICE_KEY) ?? "");
  const [rate, setRate] = useState(initialRate);
  const [scope, setScope] = useState<ResearchSpeechScope>(initialScope);

  const passages = useMemo(() => buildResearchSpeechPassages(report, scope), [report, scope]);
  const selectedModel = snapshot?.voices.find((model) => model.id === modelId) ?? snapshot?.voices[0] ?? null;
  const selectedVoiceProfile = snapshot?.voiceProfiles.find((profile) => profile.id === voiceProfileId)
    ?? snapshot?.voiceProfiles.find((profile) => profile.id === snapshot.defaultVoiceProfileId)
    ?? snapshot?.voiceProfiles[0]
    ?? null;
  const alignmentModel = snapshot?.transcriptionAvailable ? snapshot.transcribers[0] : undefined;

  const player = usePipelinedSpeechPlayer({
    sourceKind: "research",
    sourceId: report.id,
    passages,
    selectedVoiceModel: selectedModel,
    selectedVoiceProfile,
    alignmentModel,
    playbackRate: rate,
    initialDetail: "Checking local ComfyUI voice models...",
    onPassageChange: (passage) => onPassageChange?.(passage?.anchorId ?? null, passage?.id ?? null),
  });

  const activePassage = player.currentPassage;
  const seekCurrentPassage = useCallback((seconds: number) => {
    if (activePassage) player.playPassageFrom(activePassage.id, seconds);
  }, [activePassage, player.playPassageFrom]);

  useEffect(() => {
    const active = ["playing", "paused"].includes(player.status);
    if ((active || player.seekablePassages.length > 0) && activePassage) {
      onSpeechProgress?.({
        active,
        sourceKind: "research",
        sourceId: report.id,
        passageId: activePassage.id,
        text: activePassage.text,
        seconds: player.speechSeconds,
        duration: player.speechDuration,
        timings: player.speechTimings,
        onSeek: seekCurrentPassage,
        seekablePassages: player.seekablePassages,
        onSeekPassage: player.playPassageFrom,
      });
    } else {
      onSpeechProgress?.(null);
    }
  }, [
    activePassage,
    onSpeechProgress,
    player.speechDuration,
    player.speechSeconds,
    player.speechTimings,
    player.playPassageFrom,
    player.seekablePassages,
    player.status,
    report.id,
    seekCurrentPassage,
  ]);

  useEffect(() => {
    let active = true;
    void refresh()
      .then((next) => {
        if (!active) return;
        player.setDetail(next.detail);
        const selected = next.voices.find((model) => model.id === modelId) ?? next.voices[0];
        setModelId(selected?.id ?? "");
        if (selected) savePreference(MODEL_KEY, selected.id);
        const voice = next.voiceProfiles.find((profile) => profile.id === voiceProfileId)
          ?? next.voiceProfiles.find((profile) => profile.id === next.defaultVoiceProfileId)
          ?? next.voiceProfiles[0];
        setVoiceProfileId(voice?.id ?? "");
        if (voice) savePreference(VOICE_KEY, voice.id);
        if (next.narrationAvailable && !next.comfyReady) {
          player.setDetail("Starting the private ComfyUI voice engine in the background...");
          void prepare().then((ready) => {
            if (!active) return;
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

  const chooseVoice = (nextVoiceProfileId: string) => {
    player.clearModelCache();
    setVoiceProfileId(nextVoiceProfileId);
    savePreference(VOICE_KEY, nextVoiceProfileId);
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

  const unavailable = !snapshot?.narrationAvailable || !selectedModel || !selectedVoiceProfile;
  const statusText = player.status === "error"
    ? `ComfyUI narration stopped: ${player.error ?? "Playback failed"}`
    : !snapshot
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
        <label>Voice
          <select
            aria-label="Narration voice"
            value={selectedVoiceProfile?.id ?? ""}
            disabled={unavailable || ["preparing", "playing", "paused"].includes(player.status)}
            onChange={(event) => chooseVoice(event.target.value)}
          >
            {!snapshot?.voiceProfiles.length && <option value="">No local voice</option>}
            {snapshot?.voiceProfiles.map((profile) => <option value={profile.id} key={profile.id}>{profile.name}</option>)}
          </select>
        </label>
        <button type="button" className="speech-manage-voices" disabled={["preparing", "playing"].includes(player.status)} onClick={openVoiceLibrary}>Manage voices</button>
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
