import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import {
  AudioLines, Crop, FolderOpen, ImagePlus, Layers, LoaderCircle, Mic, Music, Plus,
  Save, Scissors, Sparkles, Trash2, UserRound, Video, X,
} from "lucide-react";
import {
  deleteMovieH3Entity, getImageProject, getLocalSpeechSnapshot, getMovie, getMusicProject,
  importImageTakeAsMovieReference, importMusicTakeAsMovieReference,
  importSpeechClipAsMovieReference, listImageProjects, listMovieImageAssets,
  listMovieReferenceAssets, listMusicProjects, movieMediaUrl,
  pickMovieReferenceFiles, placeMovieH3Take, stageMovieH3Reference, synthesizeLocalSpeech,
  upsertMovieH3Entity,
} from "../../../platform/api";
import type {
  H3AudioIntent, H3CropRect, H3EntityKind, H3ReferenceEntity, H3ReferenceMember,
  H3ReferencePopulation, H3ReferenceRole, H3RetentionLevel, H3Scene,
  H3VisualIntent, MovieProject, MovieReferenceAsset, VoiceProfile,
} from "../../../contracts/index";
import { PagedList } from "../../../shared/book/PagedList";
import { ExternalCollaborationExchange } from "../../../shared/collaboration/ExternalCollaborationExchange";
import { buildExternalCollaborationRequest, parseExternalTextResult } from "../../../shared/collaboration/externalCollaboration";
import type { ImageHandoff } from "../imageHandoff";

type UseAsset = (asset: MovieReferenceAsset) => void | Promise<void>;
type PictureRequest = { use?: UseAsset; purpose: string; direction?: string };
const AssetContext = createContext<((request: PictureRequest) => void) | undefined>(undefined);

/** Recent Image Studio projects searched for finished takes when the picker opens. */
const RECENT_IMAGE_PROJECTS = 24;

/** Opens the picture picker for one purpose (a guide frame or @Entity member) of the production. */
export function GenerateAssetButton({ onUse, disabled, label = "Create image", direction }: {
  onUse?: UseAsset; disabled?: boolean; label?: string; direction?: string;
}) {
  const open = useContext(AssetContext);
  if (!open) return null;
  return (
    <button
      type="button"
      className="quiet-button compact"
      disabled={disabled}
      onClick={() => open({ use: onUse, purpose: label.replace(/^Create /, ""), direction })}
    >
      <ImagePlus size={15} /> {label}
    </button>
  );
}

type Choice = {
  key: string;
  name: string;
  source: string;
  path: string;
  take?: { projectId: string; takeId: string };
  asset?: MovieReferenceAsset;
};

/**
 * Pictures for a production come from Image Studio. The picker shows its finished takes (and stills
 * made earlier by the retired H3 pass); "Make it in Image Studio" opens that chapter with a new
 * project, and the take the producer chooses there comes back to the same frame or reference.
 */
export function MovieAssetCreator({ children, production, onUse, onRequestImage, onError }: {
  children: ReactNode;
  production: string;
  onUse: UseAsset;
  onRequestImage?: (handoff: ImageHandoff) => void;
  onError: (message: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [purpose, setPurpose] = useState("Image");
  const [direction, setDirection] = useState("");
  const [choices, setChoices] = useState<Choice[]>([]);
  const [loading, setLoading] = useState(false);
  const [using, setUsing] = useState(false);
  const choose = useRef<UseAsset>(onUse);
  const callbacks = useRef({ onUse, onError });
  callbacks.current = { onUse, onError };

  const loadChoices = async () => {
    setLoading(true);
    try {
      const [summaries, stills] = await Promise.all([listImageProjects(), listMovieImageAssets()]);
      const recent = summaries.filter((summary) => summary.takeCount > 0)
        .sort((left, right) => right.updatedAt.localeCompare(left.updatedAt))
        .slice(0, RECENT_IMAGE_PROJECTS);
      const projects = await Promise.all(recent.map((summary) => getImageProject(summary.id).catch(() => undefined)));
      const takes: Choice[] = projects.flatMap((project) => project ? project.takes.flatMap((take, index) => take.status === "complete"
        ? [{ key: `take-${take.id}`, name: `${project.title} · take ${index + 1}`, source: "Image Studio", path: take.path, take: { projectId: project.id, takeId: take.id } }]
        : []) : []);
      const earlier: Choice[] = stills.filter((generation) => generation.status === "complete").flatMap((generation) => generation.candidates.map((candidate) => ({
        key: `still-${generation.id}-${candidate.frameIndex}`, name: candidate.asset.name || generation.prompt, source: "Earlier H3 still", path: candidate.asset.path, asset: candidate.asset,
      })));
      setChoices([...takes, ...earlier]);
    } catch (error) {
      callbacks.current.onError(String(error));
    } finally {
      setLoading(false);
    }
  };

  const request = (next: PictureRequest) => {
    choose.current = next.use ?? callbacks.current.onUse;
    setPurpose(next.purpose || "Image");
    setDirection(next.direction?.trim() ?? "");
    setOpen(true);
    void loadChoices();
  };

  const useChoice = async (choice: Choice) => {
    setUsing(true);
    try {
      const asset = choice.asset ?? await importImageTakeAsMovieReference(choice.take!.projectId, choice.take!.takeId);
      await choose.current(asset);
      setOpen(false);
    } catch (error) {
      callbacks.current.onError(String(error));
    } finally {
      setUsing(false);
    }
  };

  const makeInImageStudio = () => {
    if (!onRequestImage) return;
    const use = choose.current;
    onRequestImage({
      purpose,
      production,
      direction,
      deliver: async (imageProjectId, takeId) => { await use(await importImageTakeAsMovieReference(imageProjectId, takeId)); },
    });
    setOpen(false);
  };

  return (
    <AssetContext.Provider value={request}>
      {children}
      {open && (
        <div className="movie-asset-backdrop">
          <section
            className="movie-asset-dialog"
            role="dialog"
            aria-modal="true"
            aria-label="Choose a picture"
            onKeyDown={(event) => {
              if (event.key === "Escape" && !using) { event.preventDefault(); setOpen(false); }
              if (event.key === "Tab") {
                const controls = event.currentTarget.querySelectorAll<HTMLElement>("button:not(:disabled),textarea:not(:disabled),input:not(:disabled),select:not(:disabled)");
                const first = controls[0], last = controls[controls.length - 1];
                if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
                else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
              }
            }}
          >
            <header>
              <span>
                <span className="eyebrow">{purpose} · {production}</span>
                <h2>Choose a picture</h2>
                <p>Use a finished Image Studio take, or make a new picture there. It comes back to this {purpose.toLowerCase()} when you choose it.</p>
              </span>
              <button className="icon-button" aria-label="Close picture picker" disabled={using} onClick={() => setOpen(false)}>
                <X />
              </button>
            </header>
            <div className="movie-asset-create">
              <label>
                What should the picture show?
                <textarea
                  autoFocus
                  aria-label="Picture direction"
                  maxLength={65536}
                  value={direction}
                  onChange={(event) => setDirection(event.target.value)}
                  placeholder="A weathered brass compass on a dark oak desk, soft morning window light…"
                />
              </label>
              <button className="primary-button" disabled={using || !onRequestImage} onClick={makeInImageStudio}>
                <ImagePlus size={16} /> Make it in Image Studio
              </button>
              <p className="movie-asset-note">
                Image Studio opens a new project with this description. When a take is ready, choose <strong>Use in {production}</strong> there and it returns here.
              </p>
            </div>
            <div className="movie-asset-library">
              <span className="eyebrow">Finished pictures · {loading ? "…" : choices.length}</span>
              {loading ? (
                <p className="movie-asset-note"><LoaderCircle className="spin" size={14} /> Looking through Image Studio…</p>
              ) : (
                <PagedList
                  className="movie-asset-candidates"
                  label="Finished pictures"
                  items={choices}
                  itemKey={(choice) => choice.key}
                  empty={<p className="movie-asset-note">No finished pictures yet. Make one in Image Studio.</p>}
                  renderItem={(choice) => (
                    <button disabled={using} onClick={() => void useChoice(choice)} aria-label={`Use ${choice.name}`}>
                      <img src={movieMediaUrl(choice.path) || undefined} alt="" loading="lazy" />
                      <span><strong>{choice.name}</strong><small>{choice.source}</small></span>
                    </button>
                  )}
                />
              )}
            </div>
          </section>
        </div>
      )}
    </AssetContext.Provider>
  );
}

const ENTITY_KINDS: { value: H3EntityKind; label: string }[] = [
  { value: "character", label: "Character" },
  { value: "location", label: "Location" },
  { value: "prop", label: "Prop" },
  { value: "outfit", label: "Outfit" },
  { value: "voice", label: "Voice" },
  { value: "sound", label: "Sound" },
  { value: "other", label: "Other" },
];

const RETENTION_LEVELS: { value: H3RetentionLevel; label: string }[] = [
  { value: "fullyPreserved", label: "Fully Preserved (Strict Identity)" },
  { value: "partiallyPreserved", label: "Partially Preserved (Flexible)" },
  { value: "reference", label: "Reference (Balanced)" },
  { value: "attributeTransfer", label: "Attribute Transfer (Outfit / Style)" },
  { value: "weakReference", label: "Weak Reference (Loose Motif)" },
  { value: "fullyCopy", label: "Fully Copy (Exact Match)" },
];

const MEMBER_ROLES: { value: H3ReferenceRole; label: string }[] = [
  { value: "subject", label: "Subject" },
  { value: "outfit", label: "Outfit" },
  { value: "scene", label: "Scene / Location" },
  { value: "motion", label: "Motion" },
  { value: "camera", label: "Camera" },
  { value: "voice", label: "Voice" },
  { value: "sound", label: "Sound" },
  { value: "general", label: "General" },
];

const VISUAL_INTENTS: { value: H3VisualIntent; label: string }[] = [
  { value: "preserve", label: "Preserve Identity" },
  { value: "adapt", label: "Adapt to Scene" },
  { value: "transfer", label: "Transfer Attribute / Outfit" },
  { value: "context", label: "Scene / Environment Context" },
];

const AUDIO_INTENTS: { value: H3AudioIntent; label: string }[] = [
  { value: "voiceIdentity", label: "Voice Timbre Identity" },
  { value: "soundTexture", label: "Foley / Sound Texture" },
  { value: "useEmbedded", label: "Use Embedded Video Audio" },
  { value: "ignoreEmbedded", label: "Ignore Embedded Audio" },
];

function inferPopulation(kind: string): H3ReferencePopulation {
  if (kind === "video") return "videos";
  if (kind === "audio") return "standaloneAudios";
  return "pictures";
}

function defaultRoleForKind(entityKind: H3EntityKind, assetKind: string): H3ReferenceRole {
  if (assetKind === "audio") {
    return entityKind === "character" || entityKind === "voice" ? "voice" : "sound";
  }
  if (assetKind === "video") {
    return entityKind === "character" ? "subject" : "motion";
  }
  switch (entityKind) {
    case "character": return "subject";
    case "location": return "scene";
    case "outfit": return "outfit";
    case "voice": return "voice";
    case "sound": return "sound";
    default: return "general";
  }
}

function makeDraftEntity(): H3ReferenceEntity {
  return {
    id: "",
    handle: "Character",
    displayName: "New Character",
    kind: "character",
    subjectDescription: "",
    retentionLevel: "fullyPreserved",
    defaultVisualIntent: "preserve",
    defaultAudioIntent: "voiceIdentity",
    members: [],
    createdAt: "",
    updatedAt: "",
  };
}

function newMemberId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? `mem-${crypto.randomUUID().slice(0, 8)}`
    : `mem-${Date.now().toString(36)}`;
}

type MusicChoice = { projectId: string; takeId: string; label: string; durationSec: number };

export function H3EntityLibrary({
  project,
  scene,
  disabled,
  onPersistSceneIfNeeded,
  onProject,
  onDraftDescription,
  onError,
  onNotice,
}: {
  project: MovieProject;
  scene: H3Scene;
  disabled?: boolean;
  onPersistSceneIfNeeded?: () => Promise<void>;
  onProject: (next: MovieProject) => void;
  onDraftDescription?: (currentText: string, handle: string, apply: (text: string) => void) => void;
  onError: (message: string) => void;
  onNotice?: (message: string) => void;
}) {
  const [selectedEntityId, setSelectedEntityId] = useState<string>(project.entities[0]?.id ?? "__new__");
  const [draft, setDraft] = useState<H3ReferenceEntity>(() => project.entities[0] ? structuredClone(project.entities[0]) : makeDraftEntity());
  const [assets, setAssets] = useState<MovieReferenceAsset[]>([]);
  const [musicChoices, setMusicChoices] = useState<MusicChoice[]>([]);
  const [voiceProfiles, setVoiceProfiles] = useState<VoiceProfile[]>([]);
  const [speechModelId, setSpeechModelId] = useState("");
  const [speechVoice, setSpeechVoice] = useState("");
  const [speechText, setSpeechText] = useState("");
  const [speechLabel, setSpeechLabel] = useState("");
  const [speechPathInput, setSpeechPathInput] = useState("");
  const [vaultTab, setVaultTab] = useState<"library" | "music" | "speech">("library");
  const [working, setWorking] = useState(false);

  const loadVault = useCallback(async () => {
    try {
      const nextAssets = await listMovieReferenceAssets();
      setAssets(nextAssets);
    } catch {
      // Fallback to project.references when preview or empty
    }
  }, []);

  useEffect(() => { void loadVault(); }, [loadVault, project.references.length]);

  useEffect(() => {
    if (selectedEntityId === "__new__") return;
    const found = project.entities.find((e) => e.id === selectedEntityId);
    if (found) {
      setDraft(structuredClone(found));
    } else if (project.entities[0]) {
      setSelectedEntityId(project.entities[0].id);
      setDraft(structuredClone(project.entities[0]));
    } else {
      setSelectedEntityId("__new__");
      setDraft(makeDraftEntity());
    }
  }, [project.entities, selectedEntityId]);

  const loadCrossStudioSources = async (tab: "music" | "speech") => {
    setVaultTab(tab);
    if (tab === "music") {
      try {
        const summaries = await listMusicProjects();
        const projects = await Promise.all(
          summaries.slice(0, 16).map((s) => getMusicProject(s.id).catch(() => undefined)),
        );
        const found: MusicChoice[] = projects.flatMap((mp) =>
          mp
            ? mp.takes
                .filter((t) => t.status === "complete")
                .map((t, idx) => ({
                  projectId: mp.id,
                  takeId: t.id,
                  label: `${mp.title} · take ${idx + 1}`,
                  durationSec: t.durationSeconds,
                }))
            : [],
        );
        setMusicChoices(found);
      } catch (err) {
        onError(String(err));
      }
    } else if (tab === "speech") {
      try {
        const snap = await getLocalSpeechSnapshot();
        setVoiceProfiles(snap.voiceProfiles);
        setSpeechModelId(snap.voices[0]?.id || "");
        if (!speechVoice) {
          setSpeechVoice(snap.defaultVoiceProfileId || snap.voiceProfiles[0]?.id || "");
        }
      } catch {
        // Optional in browser preview
      }
    }
  };

  const mergedAssets: MovieReferenceAsset[] = (() => {
    const map = new Map<string, MovieReferenceAsset>();
    for (const a of assets) map.set(a.id, a);
    for (const ref of project.references) {
      if (!map.has(ref.assetId)) {
        map.set(ref.assetId, {
          id: ref.assetId,
          name: ref.name,
          kind: ref.kind,
          mimeType: ref.mimeType,
          bytes: ref.bytes,
          durationSeconds: ref.durationSeconds,
          width: ref.width,
          height: ref.height,
          hasAudio: ref.hasAudio,
          path: ref.path,
          createdAt: project.updatedAt,
          generation: ref.generation,
        });
      }
    }
    return Array.from(map.values());
  })();

  const appendMemberFromAsset = (asset: MovieReferenceAsset) => {
    const member: H3ReferenceMember = {
      id: newMemberId(),
      assetId: asset.id,
      label: asset.name,
      population: inferPopulation(asset.kind),
      role: defaultRoleForKind(draft.kind, asset.kind),
      sourceStartSec: 0,
      sourceEndSec: asset.kind === "image" ? 0 : Math.min(15, asset.durationSeconds || 5),
      useEmbeddedAudio: asset.kind === "video" && asset.hasAudio,
      descriptionOverride: "",
    };
    setDraft((curr) => ({ ...curr, members: [...curr.members, member] }));
  };

  const handleSaveEntity = async () => {
    setWorking(true);
    try {
      const next = await upsertMovieH3Entity({
        projectId: project.id,
        entity: draft,
      });
      onProject(next);
      const saved = next.entities.find(
        (e) => e.handle.toLowerCase() === draft.handle.trim().replace(/^@/, "").toLowerCase(),
      );
      if (saved) setSelectedEntityId(saved.id);
      onNotice?.(`Saved @${saved?.handle ?? draft.handle} to Reference Library.`);
    } catch (err) {
      onError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const handleDeleteEntity = async (entityId: string) => {
    setWorking(true);
    try {
      const next = await deleteMovieH3Entity({ projectId: project.id, entityId });
      onProject(next);
      setSelectedEntityId(next.entities[0]?.id ?? "__new__");
    } catch (err) {
      onError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const handleStageEntity = async (handle: string) => {
    const ent = project.entities.find((e) => e.handle.toLowerCase() === handle.trim().replace(/^@/, "").toLowerCase());
    if (!ent) {
      onError(`Save @${handle} first before staging onto the scene timeline.`);
      return;
    }
    setWorking(true);
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
      const updated = await getMovie(project.id);
      onProject(updated);
      onNotice?.(
        `Staged @${ent.handle} on ${receipt.laneName} [${receipt.startSec.toFixed(2)}s..${receipt.endSec.toFixed(2)}s].`,
      );
    } catch (err) {
      onError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const handleStageRawAsset = async (asset: MovieReferenceAsset) => {
    setWorking(true);
    try {
      await onPersistSceneIfNeeded?.();
      const receipt = await stageMovieH3Reference({
        projectId: project.id,
        sceneId: scene.id,
        assetId: asset.id,
        startSec: scene.window.inSec,
        endSec: Math.max(scene.window.inSec + 0.5, scene.window.outSec),
        useVisual: asset.kind !== "audio",
        useAudio: asset.kind === "audio" || (asset.kind === "video" && asset.hasAudio),
      });
      const updated = await getMovie(project.id);
      onProject(updated);
      onNotice?.(`Staged "${asset.name}" on ${receipt.laneName}.`);
    } catch (err) {
      onError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const handlePlaceAssetOnLane = async (asset: MovieReferenceAsset, laneKind: "driver" | "video" | "audio") => {
    setWorking(true);
    try {
      await onPersistSceneIfNeeded?.();
      const updated = await placeMovieH3Take({
        projectId: project.id,
        jobId: "",
        assetId: asset.id,
        targetSceneId: scene.id,
        placementMode: "trimmed",
        targetVideoLaneId: laneKind === "video" ? (scene.videoLanes[0]?.id ?? null) : null,
        targetAudioLaneId: laneKind === "audio" ? (scene.audioLanes[0]?.id ?? null) : null,
        targetDriverLaneId: laneKind === "driver" ? (scene.driverLanes[0]?.id ?? null) : null,
        timelineStartOverrideSec: scene.window.inSec,
        durationOverrideSec: Math.max(0.5, scene.window.outSec - scene.window.inSec),
      });
      onProject(updated);
      const laneLabel = laneKind === "driver" ? "D1 (Driver)" : laneKind === "video" ? "V1 (Video)" : "A1 (Audio)";
      onNotice?.(`Placed "${asset.name}" onto ${laneLabel} at ${scene.window.inSec.toFixed(2)}s.`);
    } catch (err) {
      onError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const handlePickFiles = async () => {
    setWorking(true);
    try {
      const imported = await pickMovieReferenceFiles();
      if (imported.failures.length > 0) {
        onError(imported.failures.join("\n"));
      }
      if (imported.references.length > 0) {
        setAssets((curr) => [...imported.references, ...curr.filter((c) => !imported.references.some((r) => r.id === c.id))]);
        for (const asset of imported.references) {
          appendMemberFromAsset(asset);
        }
      }
    } catch (err) {
      onError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const handleImportMusicTake = async (choice: MusicChoice) => {
    setWorking(true);
    try {
      const asset = await importMusicTakeAsMovieReference(choice.projectId, choice.takeId);
      setAssets((curr) => [asset, ...curr.filter((c) => c.id !== asset.id)]);
      appendMemberFromAsset(asset);
      onNotice?.(`Imported Music Studio take "${asset.name}".`);
    } catch (err) {
      onError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const handleImportSpeechPath = async () => {
    if (!speechPathInput.trim()) return;
    setWorking(true);
    try {
      const asset = await importSpeechClipAsMovieReference(speechPathInput.trim(), speechLabel.trim() || undefined);
      setAssets((curr) => [asset, ...curr.filter((c) => c.id !== asset.id)]);
      appendMemberFromAsset(asset);
      setSpeechPathInput("");
      onNotice?.(`Imported Speech clip "${asset.name}".`);
    } catch (err) {
      onError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const handleSynthesizeAndImportSpeech = async () => {
    if (!speechText.trim()) return;
    setWorking(true);
    try {
      const clip = await synthesizeLocalSpeech({
        jobId: `movie-voice-${Date.now()}`,
        sourceKind: "task",
        sourceId: project.id,
        passageId: `voice-${Date.now()}`,
        text: speechText.trim(),
        modelId: speechModelId || "chatterbox",
        voiceProfileId: speechVoice,
      });
      const asset = await importSpeechClipAsMovieReference(
        clip.relativePath,
        speechLabel.trim() || `@${draft.handle} Voice`,
      );
      setAssets((curr) => [asset, ...curr.filter((c) => c.id !== asset.id)]);
      appendMemberFromAsset(asset);
      onNotice?.(`Synthesized and attached voice reference "${asset.name}".`);
    } catch (err) {
      onError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const updateMember = (id: string, patch: Partial<H3ReferenceMember>) => {
    setDraft((curr) => ({
      ...curr,
      members: curr.members.map((m) => (m.id === id ? { ...m, ...patch } : m)),
    }));
  };

  const toggleMemberCrop = (member: H3ReferenceMember) => {
    if (member.crop) {
      updateMember(member.id, { crop: undefined });
    } else {
      const initialCrop: H3CropRect = { x: 0.1, y: 0.1, width: 0.8, height: 0.8 };
      updateMember(member.id, { crop: initialCrop });
    }
  };

  return (
    <section className="h3-entity-library" aria-label="H3 @Handle Reference Library">
      <aside className="h3-entity-sidebar">
        <header>
          <div>
            <strong><UserRound size={14} /> @Handle Entities ({project.entities.length})</strong>
            <small>Write @Handle in any shot; Kestrel binds &lt;Subject N&gt; automatically.</small>
          </div>
          <button
            type="button"
            disabled={disabled || working}
            onClick={() => { setSelectedEntityId("__new__"); setDraft(makeDraftEntity()); }}
          >
            <Plus size={13} /> New @Entity
          </button>
        </header>

        <div className="h3-entity-list">
          {project.entities.length === 0 && (
            <p className="h3-empty-note">
              No @Handle entities yet. Create @Character, @Location, @Prop, or @Voice entities so their descriptions and multi-modal media travel across every window.
            </p>
          )}
          {project.entities.map((entity) => (
            <article
              key={entity.id}
              className={`h3-entity-card ${selectedEntityId === entity.id ? "active" : ""}`}
            >
              <button
                type="button"
                className="h3-entity-select"
                onClick={() => setSelectedEntityId(entity.id)}
              >
                <strong>@{entity.handle}</strong>
                <span>{entity.displayName} · {entity.kind}</span>
                <small>{entity.members.length} media · {entity.retentionLevel}</small>
              </button>
              <div className="h3-entity-card-actions">
                <button
                  type="button"
                  disabled={disabled || working}
                  title={`Stage @${entity.handle} across [${scene.window.inSec.toFixed(2)}s..${scene.window.outSec.toFixed(2)}s]`}
                  onClick={() => void handleStageEntity(entity.handle)}
                >
                  <Layers size={12} /> Stage [{scene.window.inSec.toFixed(1)}–{scene.window.outSec.toFixed(1)}s]
                </button>
                <button
                  type="button"
                  className="danger-icon"
                  aria-label={`Delete @${entity.handle}`}
                  disabled={disabled || working}
                  onClick={() => void handleDeleteEntity(entity.id)}
                >
                  <Trash2 size={12} />
                </button>
              </div>
            </article>
          ))}
        </div>
      </aside>

      <div className="h3-entity-editor">
        <header className="h3-editor-header">
          <div>
            <span className="eyebrow">{draft.id ? "Edit @Entity" : "New @Entity"}</span>
            <h3>@{draft.handle.replace(/^@/, "") || "Handle"}</h3>
          </div>
          <div className="h3-editor-actions">
            {draft.id && (
              <button
                type="button"
                disabled={disabled || working}
                onClick={() => void handleStageEntity(draft.handle)}
              >
                <Layers size={14} /> Stage in [{scene.window.inSec.toFixed(2)}s..{scene.window.outSec.toFixed(2)}s]
              </button>
            )}
            <button
              type="button"
              className="primary-button"
              disabled={disabled || working || !draft.handle.trim()}
              onClick={() => void handleSaveEntity()}
            >
              <Save size={14} /> Save @{draft.handle.replace(/^@/, "") || "Entity"}
            </button>
          </div>
        </header>

        <div className="h3-entity-form-grid">
          <label>
            @Handle (ASCII)
            <input
              aria-label="Entity handle"
              value={draft.handle}
              disabled={disabled || working}
              onChange={(e) => setDraft((c) => ({ ...c, handle: e.target.value.replace(/^@/, "") }))}
              placeholder="Cowboy"
            />
          </label>
          <label>
            Display Name
            <input
              aria-label="Entity display name"
              value={draft.displayName}
              disabled={disabled || working}
              onChange={(e) => setDraft((c) => ({ ...c, displayName: e.target.value }))}
              placeholder="Weathered Rancher"
            />
          </label>
          <label>
            Entity Kind
            <select
              aria-label="Entity kind"
              value={draft.kind}
              disabled={disabled || working}
              onChange={(e) => setDraft((c) => ({ ...c, kind: e.target.value as H3EntityKind }))}
            >
              {ENTITY_KINDS.map((k) => <option key={k.value} value={k.value}>{k.label}</option>)}
            </select>
          </label>
          <label>
            Retention Level
            <select
              aria-label="Entity retention level"
              value={draft.retentionLevel}
              disabled={disabled || working}
              onChange={(e) => setDraft((c) => ({ ...c, retentionLevel: e.target.value as H3RetentionLevel }))}
            >
              {RETENTION_LEVELS.map((r) => <option key={r.value} value={r.value}>{r.label}</option>)}
            </select>
          </label>
          <label>
            Default Visual Intent
            <select
              aria-label="Entity visual intent"
              value={draft.defaultVisualIntent}
              disabled={disabled || working}
              onChange={(e) => setDraft((c) => ({ ...c, defaultVisualIntent: e.target.value as H3VisualIntent }))}
            >
              {VISUAL_INTENTS.map((vi) => <option key={vi.value} value={vi.value}>{vi.label}</option>)}
            </select>
          </label>
          <label>
            Default Audio Intent
            <select
              aria-label="Entity audio intent"
              value={draft.defaultAudioIntent}
              disabled={disabled || working}
              onChange={(e) => setDraft((c) => ({ ...c, defaultAudioIntent: e.target.value as H3AudioIntent }))}
            >
              {AUDIO_INTENTS.map((ai) => <option key={ai.value} value={ai.value}>{ai.label}</option>)}
            </select>
          </label>
          <label className="wide">
            <span>
              Subject / Timbre / Environment Description (auto-injected into H3 Channel 2 Subjects Ledger whenever staged)
            </span>
            <textarea
              aria-label="Entity subject description"
              rows={3}
              value={draft.subjectDescription}
              disabled={disabled || working}
              onChange={(e) => setDraft((c) => ({ ...c, subjectDescription: e.target.value }))}
              placeholder="A weathered 45-year-old cowboy in a dust-stained tan duster coat, deep gravelly baritone voice…"
            />
          </label>
        </div>

        <div className="h3-collab-row">
          {onDraftDescription && (
            <button
              type="button"
              className="quiet-button compact"
              disabled={disabled || working}
              onClick={() =>
                onDraftDescription(draft.subjectDescription, draft.handle, (text) =>
                  setDraft((c) => ({ ...c, subjectDescription: text })),
                )
              }
            >
              <Sparkles size={14} /> Draft description with Local Model
            </button>
          )}
          <ExternalCollaborationExchange
            title="Clipboard Collaborator (@Entity Description)"
            summary="Copy/paste structured @Entity description prompt"
            disabled={Boolean(disabled || working)}
            buildRequest={() =>
              buildExternalCollaborationRequest({
                target: "movie-reference-description",
                role: "MiniMax H3 Reference Subject Specialist",
                instructions: [
                  `Write a crisp, concrete visual and vocal subject description (2-4 sentences) for @${draft.handle} (${draft.kind}) suitable for MiniMax H3's Subjects Ledger.`,
                  "Avoid camera moves or plot beats.",
                ],
                context: {
                  projectTitle: project.title,
                  handle: `@${draft.handle}`,
                  kind: draft.kind,
                  currentDescription: draft.subjectDescription,
                },
                resultTemplate: { text: "Concise physical appearance, wardrobe, texture, and voice timbre description." },
              })
            }
            parseResponse={(text) => parseExternalTextResult(text, "movie-reference-description", 2000)}
            onApply={(text) => setDraft((c) => ({ ...c, subjectDescription: text }))}
          />
        </div>

        <section className="h3-member-section">
          <header>
            <strong>Attached Media Members ({draft.members.length})</strong>
            <div>
              <GenerateAssetButton
                label="Create Picture Reference"
                direction={draft.subjectDescription || `${draft.displayName} (${draft.kind})`}
                disabled={disabled || working}
                onUse={(asset) => {
                  setAssets((curr) => [asset, ...curr.filter((c) => c.id !== asset.id)]);
                  appendMemberFromAsset(asset);
                }}
              />
              <button type="button" className="quiet-button compact" disabled={disabled || working} onClick={() => void handlePickFiles()}>
                <FolderOpen size={14} /> Attach Local Files
              </button>
            </div>
          </header>

          {draft.members.length === 0 ? (
            <p className="h3-empty-note">
              Attach portraits, outfit stills, motion videos, or voice/SFX clips from the Vault on the right. One @Character can hold both face portraits (&lt;Picture&gt;) and a voice clip (&lt;Audio&gt;) simultaneously!
            </p>
          ) : (
            <div className="h3-member-list">
              {draft.members.map((member) => {
                const asset = mergedAssets.find((a) => a.id === member.assetId);
                return (
                  <article key={member.id} className="h3-member-card">
                    <div className="h3-member-head">
                      <div>
                        <strong>{member.label || asset?.name || member.assetId}</strong>
                        <small>{member.population} · {asset ? `${asset.kind} (${asset.width ? `${asset.width}×${asset.height}` : `${asset.durationSeconds.toFixed(1)}s`})` : member.assetId}</small>
                      </div>
                      <div>
                        {member.population !== "standaloneAudios" && (
                          <button type="button" className="quiet-button compact" onClick={() => toggleMemberCrop(member)}>
                            <Crop size={12} /> {member.crop ? "Clear Crop" : "Normalized Crop"}
                          </button>
                        )}
                        <button
                          type="button"
                          className="danger-icon"
                          aria-label={`Remove member ${member.label}`}
                          onClick={() => setDraft((c) => ({ ...c, members: c.members.filter((m) => m.id !== member.id) }))}
                        >
                          <Trash2 size={13} />
                        </button>
                      </div>
                    </div>

                    <div className="h3-member-controls">
                      <label>
                        Label
                        <input
                          value={member.label}
                          onChange={(e) => updateMember(member.id, { label: e.target.value })}
                        />
                      </label>
                      <label>
                        Role
                        <select
                          value={member.role}
                          onChange={(e) => updateMember(member.id, { role: e.target.value as H3ReferenceRole })}
                        >
                          {MEMBER_ROLES.map((r) => <option key={r.value} value={r.value}>{r.label}</option>)}
                        </select>
                      </label>
                      {member.population !== "pictures" && (
                        <>
                          <label>
                            <Scissors size={11} /> Start (s)
                            <input
                              type="number"
                              min={0}
                              step={0.1}
                              value={member.sourceStartSec}
                              onChange={(e) => updateMember(member.id, { sourceStartSec: Math.max(0, Number(e.target.value)) })}
                            />
                          </label>
                          <label>
                            End (s, ≤15s slice)
                            <input
                              type="number"
                              min={0}
                              step={0.1}
                              value={member.sourceEndSec}
                              onChange={(e) => updateMember(member.id, { sourceEndSec: Math.max(0, Number(e.target.value)) })}
                            />
                          </label>
                        </>
                      )}
                      {member.population === "videos" && (
                        <label className="checkbox-inline">
                          <input
                            type="checkbox"
                            checked={member.useEmbeddedAudio}
                            onChange={(e) => updateMember(member.id, { useEmbeddedAudio: e.target.checked })}
                          />
                          Bind Embedded Video Audio (&lt;Audio&gt;)
                        </label>
                      )}
                      <label>
                        Member Description Override (optional)
                        <input
                          value={member.descriptionOverride}
                          onChange={(e) => updateMember(member.id, { descriptionOverride: e.target.value })}
                          placeholder="Override entity description for this slot…"
                        />
                      </label>
                    </div>

                    {member.crop && (
                      <div className="h3-crop-grid">
                        <label>Crop X<input type="number" min={0} max={0.95} step={0.02} value={member.crop.x} onChange={(e) => updateMember(member.id, { crop: { ...member.crop!, x: Number(e.target.value) } })} /></label>
                        <label>Crop Y<input type="number" min={0} max={0.95} step={0.02} value={member.crop.y} onChange={(e) => updateMember(member.id, { crop: { ...member.crop!, y: Number(e.target.value) } })} /></label>
                        <label>Crop W<input type="number" min={0.05} max={1} step={0.02} value={member.crop.width} onChange={(e) => updateMember(member.id, { crop: { ...member.crop!, width: Number(e.target.value) } })} /></label>
                        <label>Crop H<input type="number" min={0.05} max={1} step={0.02} value={member.crop.height} onChange={(e) => updateMember(member.id, { crop: { ...member.crop!, height: Number(e.target.value) } })} /></label>
                      </div>
                    )}
                  </article>
                );
              })}
            </div>
          )}
        </section>
      </div>

      <aside className="h3-asset-vault">
        <header className="h3-vault-tabs">
          <button type="button" className={vaultTab === "library" ? "active" : ""} onClick={() => setVaultTab("library")}>
            <FolderOpen size={13} /> Vault ({mergedAssets.length})
          </button>
          <button type="button" className={vaultTab === "speech" ? "active" : ""} onClick={() => void loadCrossStudioSources("speech")}>
            <Mic size={13} /> Speech
          </button>
          <button type="button" className={vaultTab === "music" ? "active" : ""} onClick={() => void loadCrossStudioSources("music")}>
            <Music size={13} /> Music
          </button>
        </header>

        {vaultTab === "library" && (
          <div className="h3-vault-body">
            <div className="h3-vault-toolbar">
              <button type="button" disabled={disabled || working} onClick={() => void handlePickFiles()}>
                <FolderOpen size={13} /> Import Files
              </button>
              <GenerateAssetButton
                label="From Image Studio"
                disabled={disabled || working}
                onUse={(asset) => {
                  setAssets((curr) => [asset, ...curr.filter((c) => c.id !== asset.id)]);
                  appendMemberFromAsset(asset);
                }}
              />
            </div>
            {mergedAssets.length === 0 ? (
              <p className="h3-empty-note">Import images, videos, or audio files to bind them to @Entities or stage directly on R1..Rn.</p>
            ) : (
              <div className="h3-vault-items">
                {mergedAssets.map((asset) => (
                  <article key={asset.id} className="h3-vault-row">
                    <div>
                      <strong>
                        {asset.kind === "video" ? <Video size={12} /> : asset.kind === "audio" ? <AudioLines size={12} /> : <ImagePlus size={12} />}{" "}
                        {asset.name}
                      </strong>
                      <small>
                        {asset.kind} · {asset.width ? `${asset.width}×${asset.height}` : ""} {asset.durationSeconds > 0 ? `${asset.durationSeconds.toFixed(1)}s` : ""}
                      </small>
                    </div>
                    <div className="h3-vault-row-actions">
                      <button type="button" disabled={disabled || working} onClick={() => appendMemberFromAsset(asset)}>
                        <Plus size={12} /> Add to @{draft.handle || "Entity"}
                      </button>
                      <button type="button" disabled={disabled || working} onClick={() => void handleStageRawAsset(asset)}>
                        <Layers size={12} /> Stage R1
                      </button>
                      {asset.kind === "video" && (
                        <>
                          <button type="button" disabled={disabled || working} onClick={() => void handlePlaceAssetOnLane(asset, "driver")}>
                            + D1 Driver
                          </button>
                          <button type="button" disabled={disabled || working} onClick={() => void handlePlaceAssetOnLane(asset, "video")}>
                            + V1 Clip
                          </button>
                        </>
                      )}
                      {asset.kind === "image" && (
                        <button type="button" disabled={disabled || working} onClick={() => void handlePlaceAssetOnLane(asset, "video")}>
                          + V1 Still
                        </button>
                      )}
                      {asset.kind === "audio" && (
                        <button type="button" disabled={disabled || working} onClick={() => void handlePlaceAssetOnLane(asset, "audio")}>
                          + A1 Audio
                        </button>
                      )}
                    </div>
                  </article>
                ))}
              </div>
            )}
          </div>
        )}

        {vaultTab === "speech" && (
          <div className="h3-vault-body">
            <label>
              Voice Profile (Chatterbox TTS)
              <select value={speechVoice} onChange={(e) => setSpeechVoice(e.target.value)}>
                <option value="">Default Local Voice</option>
                {voiceProfiles.map((vp) => <option key={vp.id} value={vp.id}>{vp.name}</option>)}
              </select>
            </label>
            <label>
              Reference Label
              <input value={speechLabel} onChange={(e) => setSpeechLabel(e.target.value)} placeholder={`@${draft.handle} Voice Sample`} />
            </label>
            <label>
              Sample Line to Synthesize (locks vocal timbre for &lt;Audio N&gt;)
              <textarea
                rows={3}
                value={speechText}
                onChange={(e) => setSpeechText(e.target.value)}
                placeholder="Keep your eyes on the ridge. That storm is moving fast."
              />
            </label>
            <button
              type="button"
              className="primary-button"
              disabled={disabled || working || !speechText.trim()}
              onClick={() => void handleSynthesizeAndImportSpeech()}
            >
              <Mic size={14} /> Synthesize &amp; Attach Voice
            </button>

            <hr className="h3-divider" />
            <label>
              Or Import Existing Speech Cache Path (generated/*.opus)
              <input
                value={speechPathInput}
                onChange={(e) => setSpeechPathInput(e.target.value)}
                placeholder="generated/clip-abc123.opus"
              />
            </label>
            <button
              type="button"
              disabled={disabled || working || !speechPathInput.trim()}
              onClick={() => void handleImportSpeechPath()}
            >
              Import Speech Clip
            </button>
          </div>
        )}

        {vaultTab === "music" && (
          <div className="h3-vault-body">
            {musicChoices.length === 0 ? (
              <p className="h3-empty-note">No finished Music Studio takes found yet. Render a song in Music Studio to import it here as a score or H3 soundtrack reference.</p>
            ) : (
              <div className="h3-vault-items">
                {musicChoices.map((mc) => (
                  <article key={`${mc.projectId}-${mc.takeId}`} className="h3-vault-row">
                    <div>
                      <strong><Music size={12} /> {mc.label}</strong>
                      <small>{mc.durationSec.toFixed(1)}s · Music Studio</small>
                    </div>
                    <button type="button" disabled={disabled || working} onClick={() => void handleImportMusicTake(mc)}>
                      <Plus size={12} /> Import &amp; Attach
                    </button>
                  </article>
                ))}
              </div>
            )}
          </div>
        )}
      </aside>
    </section>
  );
}
