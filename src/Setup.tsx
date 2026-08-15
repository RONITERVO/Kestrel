import {
  Check, ChevronDown, CircleStop, Download, Film, FolderOpen, HardDrive,
  Library, LoaderCircle, MessageSquare, RefreshCw, Settings2, ShieldCheck,
  TriangleAlert,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import {
  cancelSetupInstall, installSetupComponent, onSetupProgress, openComfyUi, pickSetupFile,
  pickSetupFolder, saveSetupLocations,
} from "./api";
import { ModelDownloader } from "./ModelDownloader";
import type { AppSnapshot, ControlSnapshot, SetupLocations, SetupProgress } from "./types";

export function SetupConsole({ snapshot, onChanged, onError }: {
  snapshot: AppSnapshot;
  onChanged: Dispatch<SetStateAction<AppSnapshot | null>>;
  onError: (message: string) => void;
}) {
  const [speed, setSpeed] = useState(50);
  const [edition, setEdition] = useState<"compact" | "complete">("compact");
  const [busy, setBusy] = useState<string | null>(null);
  const [progress, setProgress] = useState<SetupProgress | null>(null);
  const [advanced, setAdvanced] = useState(false);
  const [locations, setLocations] = useState<SetupLocations>(() => fromSnapshot(snapshot));

  useEffect(() => setLocations(fromSnapshot(snapshot)), [snapshot.settings]);
  useEffect(() => {
    let dispose: (() => void) | undefined;
    void onSetupProgress(setProgress).then((unlisten) => { dispose = unlisten; });
    return () => dispose?.();
  }, []);

  const requiredReady = snapshot.setup.components.filter((item) => !item.optional && item.status === "ready").length;
  const installed = snapshot.setup.components.filter((item) => item.status === "ready").length;
  const components = useMemo(() => snapshot.setup.components.map((item) => item.id === "wikipedia" && edition === "complete"
    ? { ...item, downloadBytes: 52_709_000_000, detail: item.status === "ready" ? item.detail : "Complete English Wikipedia text without images (about 49.1 GB)." }
    : item), [snapshot.setup.components, edition]);

  const saveLocations = async (): Promise<AppSnapshot> => {
    const enginePath = locations.enginePath.trim();
    if (enginePath && !/(?:^|[\\/])llama-server\.exe$/i.test(enginePath)) {
      throw new Error("The model engine path must end with llama-server.exe. Choose the verified local engine before saving.");
    }
    const next = await saveSetupLocations({ ...locations, enginePath });
    onChanged(next);
    return next;
  };

  const install = async (component: string) => {
    setBusy(component);
    setProgress({ component, stage: "preparing", detail: "Checking saved files and available downloads…", downloadedBytes: 0, totalBytes: 0, bytesPerSecond: 0 });
    try {
      await saveLocations();
      const next = await installSetupComponent({ component, installRoot: locations.installRoot, wikipediaEdition: edition });
      onChanged(next);
      setLocations(fromSnapshot(next));
      setProgress(null);
    } catch (error) {
      setProgress(null);
      if (!String(error).toLowerCase().includes("paused")) onError(String(error));
    } finally {
      setBusy(null);
    }
  };

  const installEssentials = async () => {
    setBusy("essentials");
    try {
      await saveLocations();
      let next = snapshot;
      for (const component of ["assistant", "wikipedia"]) {
        if (next.setup.components.find((item) => item.id === component)?.status === "ready") continue;
        setProgress({ component, stage: "preparing", detail: `Preparing ${component}…`, downloadedBytes: 0, totalBytes: 0, bytesPerSecond: 0 });
        next = await installSetupComponent({ component, installRoot: locations.installRoot, wikipediaEdition: edition });
        onChanged(next);
      }
      setLocations(fromSnapshot(next));
      setProgress(null);
    } catch (error) {
      setProgress(null);
      if (!String(error).toLowerCase().includes("paused")) onError(String(error));
    } finally {
      setBusy(null);
    }
  };

  const openStudio = async () => {
    setBusy("studio-open");
    try { await openComfyUi(); } catch (error) { onError(String(error)); } finally { setBusy(null); }
  };

  const chooseFolder = async (field: "installRoot" | "bonsaiRoot" | "comfyRoot") => {
    try {
      const value = await pickSetupFolder();
      if (value) setLocations((current) => ({ ...current, [field]: value }));
    } catch (error) { onError(String(error)); }
  };

  const chooseFile = async (field: keyof SetupLocations, kind: string) => {
    try {
      const value = await pickSetupFile(kind);
      if (value) setLocations((current) => ({ ...current, [field]: value }));
    } catch (error) { onError(String(error)); }
  };

  return <div className="setup-console">
    <header className="setup-hero">
      <div className="setup-hero-copy">
        <span className="eyebrow">One-time private setup</span>
        <h1>{snapshot.setup.ready ? "Kestrel is ready." : "Let’s make Kestrel work on this computer."}</h1>
        <p>You do not need the app’s code or a technical helper. Kestrel can download, resume, verify, and remember every local component. After setup, your work runs offline.</p>
        <div className="setup-assurances"><span><ShieldCheck />No accounts required</span><span><RefreshCw />Interrupted downloads resume</span><span><HardDrive />Choose any drive</span></div>
      </div>
      <div className={`setup-readiness ${snapshot.setup.ready ? "ready" : "waiting"}`}>
        <strong>{installed}/{snapshot.setup.components.length}</strong>
        <span>parts ready</span>
        <small>{requiredReady}/2 essentials</small>
      </div>
    </header>

    {!snapshot.setup.ready && <section className="setup-simple-panel">
      <div><span className="eyebrow">Recommended</span><h2>Set up the essentials for me</h2><p>Installs the Bonsai assistant and a compact offline English Wikipedia. You can add the movie studio later.</p></div>
      <div className="setup-speed"><label>Internet speed<input type="number" min="1" max="10000" value={speed} onChange={(event) => setSpeed(Math.max(1, Number(event.target.value) || 1))} /><span>Mbps</span></label><small>At {speed} Mbps, essentials need roughly {formatTime(20_980_000_000, speed)} plus verification.</small></div>
      <button className="primary-button setup-main-button" disabled={!!busy} onClick={() => void installEssentials()}>{busy === "essentials" ? <LoaderCircle className="spin" /> : <Download />} Set up essentials</button>
    </section>}

    <section className="setup-location-row">
      <div><strong>Where large AI files live</strong><small>{snapshot.setup.availableBytes ? `${formatBytes(snapshot.setup.availableBytes)} free on the saved drive.` : "Change this before installing if your main drive is small."}</small></div>
      <label><input value={locations.installRoot} onChange={(event) => setLocations((current) => ({ ...current, installRoot: event.target.value }))} /><button onClick={() => void chooseFolder("installRoot")}><FolderOpen /> Browse</button></label>
    </section>

    {!snapshot.setup.gpuName && <div className="setup-hardware-note"><TriangleAlert /><div><strong>No NVIDIA graphics card was detected.</strong><span>The assistant and Wikipedia can still work, although Bonsai may be slow on a laptop processor. Movie Studio stays optional because MiniMax H3 is not practical on this hardware.</span></div></div>}
    {snapshot.setup.gpuName && <div className="setup-hardware-note compatible"><Check /><div><strong>{snapshot.setup.gpuName}</strong><span>{formatBytes(snapshot.setup.gpuMemoryBytes)} graphics memory detected. Kestrel will use the NVIDIA Bonsai engine; H3 remains an optional large install.</span></div></div>}

    <section className="setup-model-library" aria-labelledby="setup-model-library-title">
      <div className="setup-model-library-heading">
        <div><span className="eyebrow">Your model library</span><h2 id="setup-model-library-title">Add another local model</h2></div>
        <p>Paste a public Hugging Face model link. Kestrel will inspect compatible GGUF files first, then download only the quantization you choose with visible progress and safe resume.</p>
      </div>
      <ModelDownloader
        variant="setup"
        gpuTotalMib={snapshot.setup.gpuMemoryBytes / 1024 / 1024}
        onCatalogChanged={(control) => onChanged((current) => mergeSetupControlSnapshot(current, control))}
        onError={onError}
      />
    </section>

    <section className="setup-components" aria-label="Kestrel components">
      {components.map((component) => <article className={`setup-component ${component.status}`} key={component.id}>
        <div className="setup-component-icon">{component.id === "assistant" ? <MessageSquare /> : component.id === "wikipedia" ? <Library /> : component.id === "studio" ? <Film /> : <Settings2 />}</div>
        <div className="setup-component-copy"><div className="setup-component-title"><h2>{component.label}</h2><span className={`setup-state ${component.status}`}>{component.status === "ready" ? <><Check /> Ready</> : component.status === "partial" ? "Resume available" : component.optional ? "Optional" : "Needed"}</span></div><p>{component.detail}</p><small>{component.status === "ready" ? component.path : `${formatBytes(component.downloadBytes)} download · about ${formatTime(component.downloadBytes, speed)} at ${speed} Mbps`}</small>
          {component.id === "wikipedia" && component.status !== "ready" && <div className="wikipedia-choice"><button className={edition === "compact" ? "active" : ""} onClick={() => setEdition("compact")}><strong>Compact</strong><span>11.7 GB · article summaries</span></button><button className={edition === "complete" ? "active" : ""} onClick={() => setEdition("complete")}><strong>Complete text</strong><span>49.1 GB · full articles</span></button></div>}
        </div>
        <button className={component.status === "ready" ? "quiet-button" : "primary-button"} disabled={!!busy || (component.status === "ready" && component.id !== "studio")} onClick={() => component.status === "ready" && component.id === "studio" ? void openStudio() : void install(component.id)}>{busy === component.id || (busy === "studio-open" && component.id === "studio") ? <LoaderCircle className="spin" /> : component.status === "partial" ? <RefreshCw /> : component.status === "ready" && component.id === "studio" ? <Film /> : <Download />}{component.status === "ready" && component.id === "studio" ? "Open ComfyUI" : component.status === "ready" ? "Installed" : component.status === "partial" ? "Resume" : "Install"}</button>
      </article>)}
    </section>

    {progress && <section className="setup-progress" role="status" aria-live="polite">
      <div><LoaderCircle className="spin" /><span><strong>{progress.detail}</strong><small>{progress.totalBytes ? `${formatBytes(progress.downloadedBytes)} of ${formatBytes(progress.totalBytes)}${progress.bytesPerSecond ? ` · ${formatBytes(progress.bytesPerSecond)}/s` : ""}` : "Kestrel will keep completed and partial files."}</small></span></div>
      {progress.totalBytes > 0 && <div className="setup-progress-track"><span style={{ width: `${Math.min(100, progress.downloadedBytes / progress.totalBytes * 100)}%` }} /></div>}
      <button onClick={() => void cancelSetupInstall()}><CircleStop /> Pause safely</button>
    </section>}

    <button className="setup-advanced-toggle" onClick={() => setAdvanced((value) => !value)}><Settings2 /> Use existing files or choose every location <ChevronDown className={advanced ? "open" : ""} /></button>
    {advanced && <section className="setup-advanced-panel">
      <div className="setup-advanced-heading"><div><span className="eyebrow">Advanced and portable</span><h2>Use files already on this PC</h2><p>Nothing is tied to a drive letter. These saved locations remain editable in the installed app.</p></div><TriangleAlert /></div>
      <PathField label="Bonsai folder" value={locations.bonsaiRoot} onChange={(value) => setLocations((current) => ({ ...current, bonsaiRoot: value }))} onBrowse={() => void chooseFolder("bonsaiRoot")} />
      <PathField label="llama-server.exe" value={locations.enginePath} onChange={(value) => setLocations((current) => ({ ...current, enginePath: value }))} onBrowse={() => void chooseFile("enginePath", "engine")} />
      <PathField label="Wikipedia .zim" value={locations.wikipediaZimPath} onChange={(value) => setLocations((current) => ({ ...current, wikipediaZimPath: value }))} onBrowse={() => void chooseFile("wikipediaZimPath", "zim")} />
      <PathField label="kiwix-serve.exe" value={locations.kiwixServerPath} onChange={(value) => setLocations((current) => ({ ...current, kiwixServerPath: value }))} onBrowse={() => void chooseFile("kiwixServerPath", "engine")} />
      <PathField label="ComfyUI folder" value={locations.comfyRoot} onChange={(value) => setLocations((current) => ({ ...current, comfyRoot: value }))} onBrowse={() => void chooseFolder("comfyRoot")} />
      <PathField label="ffmpeg.exe" value={locations.ffmpegPath} onChange={(value) => setLocations((current) => ({ ...current, ffmpegPath: value }))} onBrowse={() => void chooseFile("ffmpegPath", "ffmpeg")} />
      <PathField label="ffprobe.exe" value={locations.ffprobePath} onChange={(value) => setLocations((current) => ({ ...current, ffprobePath: value }))} onBrowse={() => void chooseFile("ffprobePath", "ffprobe")} />
      <div className="setup-advanced-actions"><span>Kestrel validates what is actually present; saving a path never marks it ready by itself.</span><button className="primary-button" disabled={!!busy} onClick={() => void saveLocations().catch((error) => onError(String(error)))}><Check /> Save & check again</button></div>
    </section>}
  </div>;
}

export function mergeSetupControlSnapshot(
  current: AppSnapshot | null,
  control: ControlSnapshot,
): AppSnapshot | null {
  return current ? { ...current, control } : current;
}

function PathField({ label, value, onChange, onBrowse }: { label: string; value: string; onChange: (value: string) => void; onBrowse: () => void }) {
  return <label className="setup-path-field"><span>{label}</span><input value={value} onChange={(event) => onChange(event.target.value)} /><button onClick={onBrowse}><FolderOpen /> Choose</button></label>;
}

function fromSnapshot(snapshot: AppSnapshot): SetupLocations {
  return {
    installRoot: snapshot.settings.installRoot,
    bonsaiRoot: snapshot.settings.bonsaiRoot,
    enginePath: snapshot.control.settings.enginePath,
    wikipediaZimPath: snapshot.settings.wikipediaZimPath,
    kiwixServerPath: snapshot.settings.kiwixServerPath,
    comfyRoot: snapshot.settings.comfyRoot,
    ffmpegPath: snapshot.settings.ffmpegPath,
    ffprobePath: snapshot.settings.ffprobePath,
  };
}

function formatBytes(bytes: number): string {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
  return `${(bytes / 1024 ** index).toFixed(index > 2 ? 1 : 0)} ${units[index]}`;
}

function formatTime(bytes: number, megabits: number): string {
  const seconds = bytes * 8 / (Math.max(1, megabits) * 1_000_000);
  if (seconds < 60) return "under a minute";
  const hours = seconds / 3600;
  return hours < 2 ? `${Math.ceil(seconds / 60)} min` : `${hours.toFixed(hours < 10 ? 1 : 0)} hours`;
}
