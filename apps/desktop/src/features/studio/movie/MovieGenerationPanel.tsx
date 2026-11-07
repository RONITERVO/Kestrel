import { useEffect, useState } from "react";
import {
  AlertTriangle, CheckCircle2, CircleStop, Code2, Layers, MessageSquareQuote,
  Plus, Sliders, Sparkles, Trash2, Wand2,
} from "lucide-react";
import type {
  H3EnvironmentSnapshot, H3GenerationSettings, H3LoraSpec, H3ModelChoice,
  H3PassPreset, H3PromptFormat, H3PromptSection, H3Scene, H3TakePlacementMode,
  H3TaskMode, H3VocalEvent, H3VocalEventType, H3WindowCompileResult, MovieProject,
} from "../../../contracts/index";
import { stageMovieH3Reference } from "../../../platform/api";
import { ExternalCollaborationExchange } from "../../../shared/collaboration/ExternalCollaborationExchange";
import { buildExternalCollaborationRequest, parseExternalTextResult } from "../../../shared/collaboration/externalCollaboration";

export function parseTimelineSeconds(text: string): number {
  const normalized = text.trim().replace(",", ".");
  if (!/^\d+(?:\.\d+)?(?::\d{1,2}(?:\.\d+)?){0,2}$/.test(normalized)) return NaN;
  const parts = normalized.split(":").map(Number);
  if (parts.slice(0, -1).some((part) => !Number.isInteger(part))) return NaN;
  if (parts.slice(1).some((part) => part >= 60)) return NaN;
  return parts.reduce((seconds, part) => seconds * 60 + part, 0);
}

const PASS_PRESETS: { value: H3PassPreset; label: string; hint: string }[] = [
  { value: "draft02Mp", label: "Pass 1 · 0.2 MP Draft (608×352)", hint: "10 steps · Turbo LoRA 0.8 · Fast blocking & sync" },
  { value: "detail1Mp", label: "Pass 2 · 1 MP Detail (1344×768)", hint: "12 steps · Denoise 0.75 · Turbo LoRA 0.5 · Refine texture" },
  { value: "upscale1080p", label: "Pass 3 · 1080p Upscale (1920×1088)", hint: "4 steps · Denoise 0.40 · Turbo LoRA 0.5 · Crisp master" },
  { value: "custom", label: "Custom Resolution / Steps", hint: "Explicit canvas, steps, and denoise" },
];

const TASK_MODES: { value: H3TaskMode; label: string }[] = [
  { value: "standard", label: "Standard (Generate Video + Audio)" },
  { value: "keepExistingAudio", label: "Freeze Audio (Lip-Sync / Re-shoot Picture)" },
  { value: "audioOverExistingVideo", label: "Freeze Video (Foley / Voice / Score Over Picture)" },
  { value: "soundtrackPass", label: "Soundtrack Pass (64×32 Fast Audio Generation)" },
  { value: "characterSwap", label: "Character Swap (3-Channel Driver Video)" },
  { value: "headSwap", label: "Head Swap (3-Channel Driver Video)" },
];

const LANGUAGES = ["English", "Chinese", "Japanese", "Spanish", "French", "German", "Korean", "Italian", "Portuguese"];

const VOCAL_EVENT_TYPES: { value: H3VocalEventType; label: string }[] = [
  { value: "dialogue", label: "Dialogue" },
  { value: "narration", label: "Narration" },
  { value: "singing", label: "Singing" },
  { value: "shout", label: "Shout" },
  { value: "whisper", label: "Whisper" },
];

const PROMPT_FORMATS: { value: H3PromptFormat; label: string }[] = [
  { value: "h3FullReference", label: "6-Channel H3 Structured Reference" },
  { value: "h3Base", label: "H3 Base Prose (@Handles Expanded)" },
  { value: "characterSwap", label: "3-Channel Character Swap" },
  { value: "headSwap", label: "3-Channel Head Swap" },
];

function makeId(prefix: string): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? `${prefix}-${crypto.randomUUID().slice(0, 8)}`
    : `${prefix}-${Date.now().toString(36)}`;
}

export function MovieGenerationPanel({
  project,
  scene,
  environment,
  selectedShotId,
  compileResult,
  rendering,
  disabled,
  onSceneChange,
  onSelectShot,
  onStartGeneration,
  onStopGeneration,
  onDraftDirection,
  onPersistSceneIfNeeded,
  onProjectRefresh,
  onError,
  onNotice,
}: {
  project: MovieProject;
  scene: H3Scene;
  environment?: H3EnvironmentSnapshot;
  selectedShotId?: string;
  compileResult?: H3WindowCompileResult | null;
  rendering: boolean;
  disabled?: boolean;
  onSceneChange: (next: H3Scene) => void;
  onSelectShot: (id: string) => void;
  onStartGeneration: (expandQualityLadder: boolean) => Promise<void>;
  onStopGeneration: () => void;
  onDraftDirection?: (currentText: string, shotLabel: string, apply: (text: string) => void) => void;
  onPersistSceneIfNeeded?: () => Promise<void>;
  onProjectRefresh?: () => Promise<void>;
  onError: (message: string) => void;
  onNotice?: (message: string) => void;
}) {
  const [tab, setTab] = useState<"prompt" | "settings" | "compiled">("prompt");
  const [expandLadder, setExpandLadder] = useState(false);
  const [showGlobal, setShowGlobal] = useState(false);

  const overlappingShots = scene.promptSections.filter(
    (s) => s.timelineStartSec + s.durationSec > scene.window.inSec + 1e-3 && s.timelineStartSec < scene.window.outSec - 1e-3,
  );
  const activeShot: H3PromptSection | undefined =
    scene.promptSections.find((s) => s.id === selectedShotId) ??
    overlappingShots[0] ??
    scene.promptSections[0];

  useEffect(() => {
    if (activeShot && activeShot.id !== selectedShotId) {
      onSelectShot(activeShot.id);
    }
  }, [activeShot, onSelectShot, selectedShotId]);

  const updateGen = (patch: Partial<H3GenerationSettings>) => {
    onSceneChange({ ...scene, generation: { ...scene.generation, ...patch } });
  };

  const updateShot = (shotId: string, patch: Partial<H3PromptSection>) => {
    onSceneChange({
      ...scene,
      promptSections: scene.promptSections.map((s) => (s.id === shotId ? { ...s, ...patch } : s)),
    });
  };

  const upsertActiveShot = (
    patchOrFn: Partial<H3PromptSection> | ((shot: H3PromptSection) => Partial<H3PromptSection>),
  ) => {
    if (activeShot) {
      const patch = typeof patchOrFn === "function" ? patchOrFn(activeShot) : patchOrFn;
      updateShot(activeShot.id, patch);
      return;
    }
    const base: H3PromptSection = {
      id: makeId("shot"),
      label: `Shot ${scene.promptSections.length + 1}`,
      timelineStartSec: scene.window.inSec,
      durationSec: Math.max(0.5, scene.window.outSec - scene.window.inSec),
      summaryBeat: "",
      detailedAction: "",
      cameraAndLighting: "",
      soundscapeOverride: "",
      musicOverride: "",
      vocalEvents: [],
      enabled: true,
    };
    const patch = typeof patchOrFn === "function" ? patchOrFn(base) : patchOrFn;
    const created: H3PromptSection = { ...base, ...patch };
    onSceneChange({ ...scene, promptSections: [...scene.promptSections, created] });
    onSelectShot(created.id);
  };

  const insertHandleIntoShot = (handle: string) => {
    const token = `@${handle}`;
    upsertActiveShot((shot) => {
      const sep = shot.detailedAction && !shot.detailedAction.endsWith(" ") ? " " : "";
      return { detailedAction: `${shot.detailedAction}${sep}${token} ` };
    });
  };

  const stagedHandles = new Set(
    (compileResult?.ordinals.subjects ?? []).map((s) => s.handle.toLowerCase()),
  );

  const handleQuickStage = async (handle: string) => {
    const ent = project.entities.find((e) => e.handle.toLowerCase() === handle.toLowerCase());
    if (!ent) return;
    try {
      await onPersistSceneIfNeeded?.();
      const receipt = await stageMovieH3Reference({
        projectId: project.id,
        sceneId: scene.id,
        entityId: ent.id,
        startSec: scene.window.inSec,
        endSec: Math.max(scene.window.inSec + 0.5, scene.window.outSec),
        useVisual: true,
        useAudio: true,
      });
      await onProjectRefresh?.();
      onNotice?.(`Staged @${ent.handle} on ${receipt.laneName}.`);
    } catch (err) {
      onError(String(err));
    }
  };

  const addVocalEvent = () => {
    const defaultSpeaker = project.entities.find((e) => e.kind === "character" || e.kind === "voice")?.handle ?? "";
    const ev: H3VocalEvent = {
      id: makeId("vocal"),
      speakerHandle: defaultSpeaker,
      eventType: "dialogue",
      language: "English",
      delivery: "calm, close-mic",
      text: "",
    };
    upsertActiveShot((shot) => ({ vocalEvents: [...shot.vocalEvents, ev] }));
  };

  const updateVocalEvent = (shot: H3PromptSection, evId: string, patch: Partial<H3VocalEvent>) => {
    updateShot(shot.id, {
      vocalEvents: shot.vocalEvents.map((v) => (v.id === evId ? { ...v, ...patch } : v)),
    });
  };

  const isSwap =
    scene.generation.taskMode === "characterSwap" || scene.generation.taskMode === "headSwap";
  const hasBlockingError = (compileResult?.advisories ?? []).some((a) => a.severity === "error");

  return (
    <aside className="h3-generation-panel" aria-label="H3 Window Compiler and Generation">
      <header className="h3-gen-tabs">
        <button type="button" className={tab === "prompt" ? "active" : ""} onClick={() => setTab("prompt")}>
          <Wand2 size={13} /> Shots &amp; @Handles
        </button>
        <button type="button" className={tab === "settings" ? "active" : ""} onClick={() => setTab("settings")}>
          <Sliders size={13} /> Pass &amp; Mode
        </button>
        <button type="button" className={tab === "compiled" ? "active" : ""} onClick={() => setTab("compiled")}>
          <Code2 size={13} /> Compiler
          {(compileResult?.advisories.length ?? 0) > 0 && (
            <b className="h3-tab-badge">{compileResult?.advisories.length}</b>
          )}
        </button>
      </header>

      <div className="h3-gen-body">
        {tab === "prompt" && (
          <>
            {/* @Handle Quick-Insert & Stage Bar */}
            <div className="h3-handle-bar">
              <span className="eyebrow">@Handle Palette (Window [{scene.window.inSec.toFixed(1)}s–{scene.window.outSec.toFixed(1)}s])</span>
              {project.entities.length === 0 ? (
                <small className="h3-muted">Create @Character / @Location / @Prop entities in the References tab to bind automatic &lt;Subject N&gt; tags.</small>
              ) : (
                <div className="h3-handle-chips">
                  {project.entities.map((ent) => {
                    const staged = stagedHandles.has(ent.handle.toLowerCase());
                    return (
                      <span key={ent.id} className={`h3-handle-chip ${staged ? "staged" : "unstaged"}`}>
                        <button
                          type="button"
                          disabled={disabled || rendering}
                          onClick={() => insertHandleIntoShot(ent.handle)}
                          title={`Insert @${ent.handle} into shot action`}
                        >
                          @{ent.handle}
                        </button>
                        {!staged && (
                          <button
                            type="button"
                            className="stage-mini"
                            disabled={disabled || rendering}
                            onClick={() => void handleQuickStage(ent.handle)}
                            title={`Stage @${ent.handle} in [${scene.window.inSec.toFixed(1)}s..${scene.window.outSec.toFixed(1)}s]`}
                          >
                            +Stage
                          </button>
                        )}
                      </span>
                    );
                  })}
                </div>
              )}
            </div>

            {isSwap ? (
              <div className="h3-field-stack">
                <label>
                  Target in Driver Video
                  <input
                    aria-label="Swap target description"
                    value={activeShot?.swapFields?.targetSubject ?? ""}
                    onChange={(e) => {
                      const val = e.target.value;
                      upsertActiveShot((shot) => {
                        const prev = shot.swapFields ?? { targetSubject: "", replacementHandle: "", keepPoseAndLighting: true, extraInstruction: "" };
                        return { swapFields: { ...prev, targetSubject: val } };
                      });
                    }}
                    placeholder="the rider on the left in the driver video"
                  />
                </label>
                <label>
                  Replacement (@Character or &lt;Picture 1&gt;)
                  <input
                    aria-label="Swap replacement description"
                    value={activeShot?.swapFields?.replacementHandle ?? ""}
                    onChange={(e) => {
                      const val = e.target.value;
                      upsertActiveShot((shot) => {
                        const prev = shot.swapFields ?? { targetSubject: "", replacementHandle: "", keepPoseAndLighting: true, extraInstruction: "" };
                        return { swapFields: { ...prev, replacementHandle: val } };
                      });
                    }}
                    placeholder="@Cowboy wearing a weathered tan duster"
                  />
                </label>
                <label>
                  Preserved Scene &amp; Lighting
                  <textarea
                    aria-label="Swap environment description"
                    rows={3}
                    value={activeShot?.swapFields?.extraInstruction ?? ""}
                    onChange={(e) => {
                      const val = e.target.value;
                      upsertActiveShot((shot) => {
                        const prev = shot.swapFields ?? { targetSubject: "", replacementHandle: "", keepPoseAndLighting: true, extraInstruction: "" };
                        return { swapFields: { ...prev, extraInstruction: val } };
                      });
                    }}
                    placeholder="Keep the desert canyon sunset lighting, camera shake, and dust haze unchanged."
                  />
                </label>
                <label className="checkbox-inline">
                  <input
                    type="checkbox"
                    checked={activeShot?.swapFields?.keepPoseAndLighting ?? true}
                    onChange={(e) => {
                      const checked = e.target.checked;
                      upsertActiveShot((shot) => {
                        const prev = shot.swapFields ?? { targetSubject: "", replacementHandle: "", keepPoseAndLighting: true, extraInstruction: "" };
                        return { swapFields: { ...prev, keepPoseAndLighting: checked } };
                      });
                    }}
                  />
                  Preserve Driver Pose, Camera &amp; Scene Lighting
                </label>
              </div>
            ) : (
              <>
                <div className="h3-shot-selector">
                  <label>
                    Active Shot
                    <select
                      aria-label="Selected prompt shot"
                      value={activeShot?.id ?? ""}
                      onChange={(e) => onSelectShot(e.target.value)}
                    >
                      {scene.promptSections.length === 0 && <option value="">No shots yet — click + Shot</option>}
                      {scene.promptSections.map((s, idx) => (
                        <option key={s.id} value={s.id}>
                          [Shot {idx + 1}] {s.label} ({s.timelineStartSec.toFixed(1)}s–{(s.timelineStartSec + s.durationSec).toFixed(1)}s){s.enabled ? "" : " [Muted]"}
                        </option>
                      ))}
                    </select>
                  </label>
                  {activeShot && (
                    <label className="checkbox-inline" title="Enable or mute this shot beat">
                      <input
                        type="checkbox"
                        aria-label="Enable active shot"
                        checked={activeShot.enabled}
                        onChange={(e) => updateShot(activeShot.id, { enabled: e.target.checked })}
                      />
                      On
                    </label>
                  )}
                  <button
                    type="button"
                    disabled={disabled || rendering}
                    onClick={() => {
                      const created: H3PromptSection = {
                        id: makeId("shot"),
                        label: `Shot ${scene.promptSections.length + 1}`,
                        timelineStartSec: scene.window.inSec,
                        durationSec: Math.max(0.5, scene.window.outSec - scene.window.inSec),
                        summaryBeat: "",
                        detailedAction: "",
                        cameraAndLighting: "",
                        soundscapeOverride: "",
                        musicOverride: "",
                        vocalEvents: [],
                        enabled: true,
                      };
                      onSceneChange({
                        ...scene,
                        promptSections: [...scene.promptSections, created].sort((a, b) => a.timelineStartSec - b.timelineStartSec),
                      });
                      onSelectShot(created.id);
                    }}
                  >
                    <Plus size={12} /> Shot
                  </button>
                  {activeShot && (
                    <button
                      type="button"
                      className="danger-icon"
                      aria-label="Delete active shot"
                      disabled={disabled || rendering}
                      onClick={() => {
                        const remaining = scene.promptSections.filter((s) => s.id !== activeShot.id);
                        onSceneChange({ ...scene, promptSections: remaining });
                        onSelectShot(remaining[0]?.id ?? "");
                      }}
                    >
                      <Trash2 size={12} />
                    </button>
                  )}
                </div>

                {activeShot ? (
                  <div className="h3-field-stack">
                    <div className="h3-row-3">
                      <label>
                        Label
                        <input
                          aria-label="Shot label"
                          value={activeShot.label}
                          onChange={(e) => updateShot(activeShot.id, { label: e.target.value })}
                        />
                      </label>
                      <label>
                        Start (s)
                        <input
                          aria-label="Shot start seconds"
                          type="number"
                          min={0}
                          step={0.25}
                          value={activeShot.timelineStartSec}
                          onChange={(e) => updateShot(activeShot.id, { timelineStartSec: Math.max(0, Number(e.target.value)) })}
                        />
                      </label>
                      <label>
                        Dur (s)
                        <input
                          aria-label="Shot duration seconds"
                          type="number"
                          min={0.5}
                          step={0.25}
                          value={activeShot.durationSec}
                          onChange={(e) => updateShot(activeShot.id, { durationSec: Math.max(0.5, Number(e.target.value)) })}
                        />
                      </label>
                    </div>

                    <label>
                      Summary Beat (Channel 1: Overall Summary)
                      <input
                        aria-label="Shot summary beat"
                        value={activeShot.summaryBeat}
                        onChange={(e) => updateShot(activeShot.id, { summaryBeat: e.target.value })}
                        placeholder="@Cowboy steps onto the porch and scans the horizon."
                      />
                    </label>

                    <label>
                      Detailed Action (Channel 3: Visual Action with @Handles)
                      <textarea
                        aria-label="Shot detailed action"
                        rows={4}
                        value={activeShot.detailedAction}
                        onChange={(e) => updateShot(activeShot.id, { detailedAction: e.target.value })}
                        placeholder="Medium close-up on @Cowboy at @RanchPorch. Wind lifts the brim of his hat as he turns toward camera…"
                      />
                    </label>

                    <label>
                      Camera &amp; Lighting
                      <input
                        aria-label="Shot camera and lighting"
                        value={activeShot.cameraAndLighting}
                        onChange={(e) => updateShot(activeShot.id, { cameraAndLighting: e.target.value })}
                        placeholder="35mm anamorphic handheld push-in, warm golden-hour rim light, shallow DOF"
                      />
                    </label>

                    {/* Vocal Events (<d>[Language] ...</d>) */}
                    <div className="h3-vocals-box">
                      <header>
                        <strong><MessageSquareQuote size={12} /> Spoken Dialogue (&lt;d&gt;[Language] …&lt;/d&gt;)</strong>
                        <button type="button" className="quiet-button compact" onClick={addVocalEvent}>
                          <Plus size={12} /> Line
                        </button>
                      </header>
                      {activeShot.vocalEvents.map((v) => (
                        <div key={v.id} className="h3-vocal-row">
                          <div className="h3-row-4">
                            <input
                              aria-label="Vocal speaker handle"
                              value={v.speakerHandle}
                              onChange={(e) => updateVocalEvent(activeShot, v.id, { speakerHandle: e.target.value.replace(/^@/, "") })}
                              placeholder="@Speaker"
                            />
                            <select
                              aria-label="Vocal event type"
                              value={v.eventType}
                              onChange={(e) => updateVocalEvent(activeShot, v.id, { eventType: e.target.value as H3VocalEventType })}
                            >
                              {VOCAL_EVENT_TYPES.map((vt) => <option key={vt.value} value={vt.value}>{vt.label}</option>)}
                            </select>
                            <select
                              aria-label="Vocal language"
                              value={v.language}
                              onChange={(e) => updateVocalEvent(activeShot, v.id, { language: e.target.value })}
                            >
                              {LANGUAGES.map((lang) => <option key={lang} value={lang}>{lang}</option>)}
                            </select>
                            <button
                              type="button"
                              className="danger-icon"
                              aria-label="Remove vocal line"
                              onClick={() =>
                                updateShot(activeShot.id, {
                                  vocalEvents: activeShot.vocalEvents.filter((item) => item.id !== v.id),
                                })
                              }
                            >
                              <Trash2 size={12} />
                            </button>
                          </div>
                          <input
                            aria-label="Vocal performance note"
                            value={v.delivery}
                            onChange={(e) => updateVocalEvent(activeShot, v.id, { delivery: e.target.value })}
                            placeholder="low, measured tone"
                          />
                          <input
                            aria-label="Vocal spoken text"
                            value={v.text}
                            onChange={(e) => updateVocalEvent(activeShot, v.id, { text: e.target.value })}
                            placeholder="That storm isn't waiting until morning."
                          />
                        </div>
                      ))}
                    </div>

                    <div className="h3-collab-row">
                      {onDraftDirection && (
                        <button
                          type="button"
                          className="quiet-button compact"
                          disabled={disabled || rendering}
                          onClick={() =>
                            onDraftDirection(activeShot.detailedAction, activeShot.label, (text) =>
                              updateShot(activeShot.id, { detailedAction: text }),
                            )
                          }
                        >
                          <Sparkles size={13} /> Expand Beat with Local Model
                        </button>
                      )}
                      <ExternalCollaborationExchange
                        title="Clipboard Shot Collaborator"
                        summary="Copy/paste H3 shot direction"
                        disabled={Boolean(disabled || rendering)}
                        buildRequest={() =>
                          buildExternalCollaborationRequest({
                            target: "movie-generation-direction",
                            role: "MiniMax H3 Cinematographer",
                            instructions: [
                              "Write vivid physical blocking and camera action for this MiniMax H3 shot using @Handle tokens for characters/locations/props.",
                              "Do not write literal <Subject> or <Picture> tags.",
                            ],
                            context: {
                              story: project.storyMarkdown.slice(0, 3000),
                              availableHandles: project.entities.map((e) => `@${e.handle} (${e.kind}): ${e.subjectDescription}`),
                              shotLabel: activeShot.label,
                              windowSec: `${scene.window.inSec.toFixed(2)}s..${scene.window.outSec.toFixed(2)}s`,
                              currentAction: activeShot.detailedAction,
                            },
                            resultTemplate: { text: "Detailed visual action prose using @Handle tokens." },
                          })
                        }
                        parseResponse={(text) => parseExternalTextResult(text, "movie-generation-direction", 8000)}
                        onApply={(text) => updateShot(activeShot.id, { detailedAction: text })}
                      />
                    </div>
                  </div>
                ) : (
                  <p className="h3-empty-note">Click <strong>+ Shot</strong> to create a timed beat inside [{scene.window.inSec.toFixed(1)}s..{scene.window.outSec.toFixed(1)}s], or use Scene Global Prompt below.</p>
                )}

                <details className="h3-global-details" open={showGlobal} onToggle={(e) => setShowGlobal(e.currentTarget.open)}>
                  <summary>Scene Global Lead, Foley &amp; Score (Channels 1, 3, 4, 6 &amp; Negative)</summary>
                  <div className="h3-field-stack">
                    <label>
                      Global Summary Lead (Channel 1)
                      <input
                        value={scene.globalPrompt.summaryLead}
                        onChange={(e) =>
                          onSceneChange({
                            ...scene,
                            globalPrompt: { ...scene.globalPrompt, summaryLead: e.target.value },
                          })
                        }
                        placeholder="Cinematic 35mm neo-western sequence at dusk."
                      />
                    </label>
                    <label>
                      Global Detailed Lead (Channel 3 Visual Prefix)
                      <input
                        value={scene.globalPrompt.detailedLead}
                        onChange={(e) =>
                          onSceneChange({
                            ...scene,
                            globalPrompt: { ...scene.globalPrompt, detailedLead: e.target.value },
                          })
                        }
                        placeholder="Warm anamorphic lens flare, subtle film grain, natural skin texture."
                      />
                    </label>
                    <label>
                      Overall Soundscape &amp; Foley (Channel 4)
                      <input
                        value={scene.globalPrompt.overallSoundscape}
                        onChange={(e) =>
                          onSceneChange({
                            ...scene,
                            globalPrompt: { ...scene.globalPrompt, overallSoundscape: e.target.value },
                          })
                        }
                        placeholder="Dry desert wind, creaking porch timber, distant thunder rumble."
                      />
                    </label>
                    <label>
                      Non-Diegetic Music / Score (Channel 6 — leave empty for pure diegetic foley)
                      <input
                        value={scene.globalPrompt.nonDiegeticMusic}
                        onChange={(e) =>
                          onSceneChange({
                            ...scene,
                            globalPrompt: { ...scene.globalPrompt, nonDiegeticMusic: e.target.value },
                          })
                        }
                        placeholder="None (or sparse solo cello drone)"
                      />
                    </label>
                    <label>
                      Negative Prompt
                      <input
                        value={scene.globalPrompt.negativePrompt}
                        onChange={(e) =>
                          onSceneChange({
                            ...scene,
                            globalPrompt: { ...scene.globalPrompt, negativePrompt: e.target.value },
                          })
                        }
                        placeholder="worst quality, watermark, text subtitles, distorted hands"
                      />
                    </label>
                  </div>
                </details>
              </>
            )}
          </>
        )}

        {tab === "settings" && (
          <div className="h3-field-stack">
            <label>
              Task Mode
              <select
                aria-label="H3 task mode"
                value={scene.generation.taskMode}
                onChange={(e) => updateGen({ taskMode: e.target.value as H3TaskMode })}
              >
                {TASK_MODES.map((m) => <option key={m.value} value={m.value}>{m.label}</option>)}
              </select>
            </label>

            <label>
              Pass Preset
              <select
                aria-label="H3 pass preset"
                value={scene.generation.passPreset}
                onChange={(e) => updateGen({ passPreset: e.target.value as H3PassPreset })}
              >
                {PASS_PRESETS.map((p) => <option key={p.value} value={p.value}>{p.label}</option>)}
              </select>
              <small className="h3-muted">
                {PASS_PRESETS.find((p) => p.value === scene.generation.passPreset)?.hint}
              </small>
            </label>

            <label className="h3-ladder-box">
              <input
                type="checkbox"
                aria-label="Run 3-pass Sonder quality ladder"
                checked={expandLadder}
                onChange={(e) => setExpandLadder(e.target.checked)}
              />
              <span>
                <strong>Run Full 3-Pass Sonder Quality Ladder</strong>
                <small>Queues 0.2 MP Draft (10 steps) → 1 MP Detail (12 steps, 0.75 denoise) → 1080p Upscale (4 steps, 0.40 denoise) automatically.</small>
              </span>
            </label>

            {scene.generation.passPreset === "custom" && (
              <div className="h3-row-3">
                <label>
                  Width
                  <input
                    type="number"
                    step={32}
                    min={64}
                    max={1920}
                    value={scene.generation.customWidth}
                    onChange={(e) => updateGen({ customWidth: Number(e.target.value) })}
                  />
                </label>
                <label>
                  Height
                  <input
                    type="number"
                    step={32}
                    min={32}
                    max={1920}
                    value={scene.generation.customHeight}
                    onChange={(e) => updateGen({ customHeight: Number(e.target.value) })}
                  />
                </label>
                <label>
                  Steps
                  <input
                    type="number"
                    min={1}
                    max={50}
                    value={scene.generation.steps}
                    onChange={(e) => updateGen({ steps: Number(e.target.value) })}
                  />
                </label>
              </div>
            )}

            <div className="h3-row-2">
              <label>
                UNET Pathway
                <select
                  aria-label="H3 model choice"
                  value={scene.generation.modelChoice}
                  onChange={(e) => updateGen({ modelChoice: e.target.value as H3ModelChoice })}
                >
                  <option value="auto">Auto (Ref2VA w/ refs, FL2VA w/o)</option>
                  <option value="ref2va">MiniMax H3 Ref2VA</option>
                  <option value="fl2va">MiniMax H3 FL2VA</option>
                  <option value="ref2vaCondWithFl2vaModel">Hybrid Ref2VA Cond + FL2VA</option>
                </select>
              </label>
              <label>
                Prompt Format
                <select
                  aria-label="H3 prompt format"
                  value={scene.generation.promptFormat}
                  onChange={(e) => updateGen({ promptFormat: e.target.value as H3PromptFormat })}
                >
                  {PROMPT_FORMATS.map((pf) => <option key={pf.value} value={pf.value}>{pf.label}</option>)}
                </select>
              </label>
            </div>

            <div className="h3-row-2">
              <label>
                Placement
                <select
                  aria-label="Take placement mode"
                  value={scene.generation.placementMode}
                  onChange={(e) => updateGen({ placementMode: e.target.value as H3TakePlacementMode })}
                >
                  <option value="trimmed">Trimmed to [In, Out]</option>
                  <option value="untrimmed">Untrimmed (Include Context)</option>
                  <option value="assetOnly">Save to Vault Only (No Clip Placed)</option>
                </select>
              </label>
              <label>
                Denoise (0.05–1.0)
                <input
                  type="number"
                  min={0.05}
                  max={1}
                  step={0.05}
                  value={scene.generation.denoise}
                  onChange={(e) => updateGen({ denoise: Number(e.target.value) })}
                />
              </label>
            </div>

            <div className="h3-row-2">
              <label>
                Seed (0 = random)
                <input
                  aria-label="H3 generation seed"
                  value={String(scene.generation.seed)}
                  onChange={(e) => {
                    const raw = e.target.value.trim();
                    if (/^\d+$/.test(raw)) {
                      const num = Number(raw);
                      const isZero = raw === "0";
                      updateGen({
                        seed: Number.isSafeInteger(num) ? num : raw,
                        randomizeSeed: isZero,
                      });
                    }
                  }}
                />
              </label>
              <label className="checkbox-inline" style={{ alignSelf: "end", paddingBottom: "6px" }}>
                <input
                  type="checkbox"
                  aria-label="Randomize H3 seed"
                  checked={scene.generation.randomizeSeed}
                  onChange={(e) => updateGen({ randomizeSeed: e.target.checked })}
                />
                Randomize Seed Each Take
              </label>
            </div>

            <label className="checkbox-inline">
              <input
                type="checkbox"
                checked={scene.generation.colorDriftCorrection}
                onChange={(e) => updateGen({ colorDriftCorrection: e.target.checked })}
              />
              Ridge-Affine Color-Drift Cancellation on Frozen Pre/Post Context
            </label>

            {/* Additional LoRA Stack */}
            <div className="h3-vocals-box">
              <header>
                <strong>Additional H3 LoRAs ({scene.generation.loras.length})</strong>
                <button
                  type="button"
                  className="quiet-button compact"
                  onClick={() => {
                    const defaultLora = environment?.discoveredLoras[0];
                    const nextLora: H3LoraSpec = {
                      fileName: defaultLora?.fileName ?? "",
                      strengthModel: defaultLora?.defaultStrength ?? 0.6,
                      enabled: true,
                    };
                    updateGen({ loras: [...scene.generation.loras, nextLora] });
                  }}
                >
                  <Plus size={12} /> LoRA
                </button>
              </header>
              {scene.generation.loras.map((lora, idx) => (
                <div key={idx} className="h3-row-3">
                  <label>
                    LoRA File
                    {environment && environment.discoveredLoras.length > 0 ? (
                      <select
                        aria-label={`LoRA ${idx + 1} file`}
                        value={lora.fileName}
                        onChange={(e) => {
                          const loras = scene.generation.loras.map((item, i) =>
                            i === idx ? { ...item, fileName: e.target.value } : item,
                          );
                          updateGen({ loras });
                        }}
                      >
                        <option value="">Select LoRA…</option>
                        {environment.discoveredLoras.map((f) => <option key={f.fileName} value={f.fileName}>{f.fileName}</option>)}
                      </select>
                    ) : (
                      <input
                        aria-label={`LoRA ${idx + 1} file`}
                        value={lora.fileName}
                        onChange={(e) => {
                          const loras = scene.generation.loras.map((item, i) =>
                            i === idx ? { ...item, fileName: e.target.value } : item,
                          );
                          updateGen({ loras });
                        }}
                        placeholder="custom_h3_style.safetensors"
                      />
                    )}
                  </label>
                  <label>
                    Strength
                    <input
                      type="number"
                      min={-2}
                      max={2}
                      step={0.05}
                      value={lora.strengthModel}
                      onChange={(e) => {
                        const loras = scene.generation.loras.map((item, i) =>
                          i === idx ? { ...item, strengthModel: Number(e.target.value) } : item,
                        );
                        updateGen({ loras });
                      }}
                    />
                  </label>
                  <button
                    type="button"
                    className="danger-icon"
                    aria-label={`Remove LoRA ${idx + 1}`}
                    onClick={() =>
                      updateGen({
                        loras: scene.generation.loras.filter((_, i) => i !== idx),
                      })
                    }
                  >
                    <Trash2 size={12} />
                  </button>
                </div>
              ))}
            </div>
          </div>
        )}

        {tab === "compiled" && (
          <div className="h3-compiler-view">
            {compileResult ? (
              <>
                <div className="h3-compile-badges">
                  <span>{compileResult.resolvedWidth}×{compileResult.resolvedHeight}</span>
                  <span>{compileResult.geometry.paddedFrames}f (17k+5)</span>
                  <span>{compileResult.resolvedSteps} steps · denoise {compileResult.resolvedDenoise.toFixed(2)}</span>
                  <span>{compileResult.resolvedModel}</span>
                </div>

                {compileResult.resolvedLoras.length > 0 && (
                  <div className="h3-compile-loras">
                    <small>LoRA Chain:</small>
                    {compileResult.resolvedLoras.map((l, i) => (
                      <code key={i}>{l.fileName} ({l.strengthModel.toFixed(2)})</code>
                    ))}
                  </div>
                )}

                {compileResult.ordinals.subjects.length > 0 && (
                  <div className="h3-compile-ordinals">
                    <span className="eyebrow">Bound &lt;Subject&gt; Ordinals</span>
                    {compileResult.ordinals.subjects.map((s) => (
                      <div key={s.subjectTag} className="h3-ordinal-row">
                        <strong>@{s.handle} → {s.subjectTag} ({s.speakerCode})</strong>
                        <small>{[...s.visualTags, ...s.audioTags].join(", ")}</small>
                      </div>
                    ))}
                  </div>
                )}

                {compileResult.advisories.length > 0 && (
                  <div className="h3-advisories">
                    {compileResult.advisories.map((adv, idx) => (
                      <p key={idx} className={`h3-advisory ${adv.severity}`}>
                        <AlertTriangle size={13} /> <span>{adv.message}</span>
                      </p>
                    ))}
                  </div>
                )}

                {compileResult.advisories.length === 0 && (
                  <p className="h3-advisory ok">
                    <CheckCircle2 size={13} /> <span>All active @Handles and H3 caps (≤9 Pictures, ≤3 Videos, ≤3 Audios) verified cleanly.</span>
                  </p>
                )}

                <label>
                  Exact Compiled {compileResult.promptFormat} Prompt
                  <pre className="h3-compiled-pre">{compileResult.compiledPrompt}</pre>
                </label>
              </>
            ) : (
              <p className="h3-empty-note">Compiling active [{scene.window.inSec.toFixed(2)}s..{scene.window.outSec.toFixed(2)}s] window…</p>
            )}
          </div>
        )}
      </div>

      <footer className="h3-gen-footer">
        {rendering ? (
          <button type="button" className="danger-button wide-btn" onClick={onStopGeneration}>
            <CircleStop size={15} /> Stop MiniMax H3 Generation
          </button>
        ) : (
          <button
            type="button"
            className="primary-button wide-btn"
            disabled={Boolean(disabled || hasBlockingError)}
            onClick={() => void onStartGeneration(expandLadder)}
          >
            <Layers size={15} />{" "}
            {expandLadder
              ? `Queue 3-Pass Quality Ladder [${scene.window.inSec.toFixed(1)}s–${scene.window.outSec.toFixed(1)}s]`
              : `Generate H3 Window [${scene.window.inSec.toFixed(1)}s–${scene.window.outSec.toFixed(1)}s]`}
          </button>
        )}
      </footer>
    </aside>
  );
}
