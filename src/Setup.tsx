import {
  Check, ChevronDown, CircleStop, Download, FileMusic, Film, FolderOpen, HardDrive, Headphones, Image as ImageIcon,
  Library, LoaderCircle, MessageSquare, Mic2, RefreshCw, Settings2, ShieldCheck,
  TriangleAlert,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import {
  cancelSetupInstall, installSetupComponent, onSetupProgress, openComfyUi, pickSetupFile,
  pickSetupFolder, saveSetupLocations, scanSetupModelFolder,
} from "./api";
import { ModelDownloader } from "./ModelDownloader";
import type { AppSnapshot, ControlSnapshot, SetupLocations, SetupProgress } from "./types";

const WHISPER_MODEL_ID = "speech:large-v3-turbo.pt";
const MUSCRIPTOR_MODEL_ID = "muscriptor:model.safetensors";

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
  const [ideogramLicenseOpen, setIdeogramLicenseOpen] = useState(false);
  const [ideogramLicenseAccepted, setIdeogramLicenseAccepted] = useState(false);
  const [muscriptorLicenseOpen, setMuscriptorLicenseOpen] = useState(false);
  const [muscriptorLicenseAccepted, setMuscriptorLicenseAccepted] = useState(false);
  const [existingModelPaths, setExistingModelPaths] = useState<Record<string, string>>({});
  const [modelScanRoot, setModelScanRoot] = useState("");
  const [modelScanMessage, setModelScanMessage] = useState("");
  const [scanningModels, setScanningModels] = useState(false);
  const [locations, setLocations] = useState<SetupLocations>(() => fromSnapshot(snapshot));
  const ideogramLicenseDialogRef = useRef<HTMLDialogElement>(null);
  const muscriptorLicenseDialogRef = useRef<HTMLDialogElement>(null);

  useEffect(() => setLocations(fromSnapshot(snapshot)), [snapshot.settings]);
  useEffect(() => {
    const dialog = ideogramLicenseDialogRef.current;
    if (!ideogramLicenseOpen || !dialog || dialog.open) return;
    if (typeof dialog.showModal === "function") dialog.showModal();
    else dialog.setAttribute("open", "");
  }, [ideogramLicenseOpen]);
  useEffect(() => {
    const dialog = muscriptorLicenseDialogRef.current;
    if (!muscriptorLicenseOpen || !dialog || dialog.open) return;
    if (typeof dialog.showModal === "function") dialog.showModal();
    else dialog.setAttribute("open", "");
  }, [muscriptorLicenseOpen]);
  useEffect(() => {
    let dispose: (() => void) | undefined;
    void onSetupProgress(setProgress).then((unlisten) => { dispose = unlisten; });
    return () => dispose?.();
  }, []);

  const requiredReady = snapshot.setup.components.filter((item) => !item.optional && item.status === "ready").length;
  const installed = snapshot.setup.components.filter((item) => item.status === "ready").length;
  const productionIds = ["media", "studio", "music", "speech"];
  const productionReady = productionIds.every((id) => snapshot.setup.components.find((item) => item.id === id)?.status === "ready");
  const productionBytes = snapshot.setup.components.filter((item) => productionIds.includes(item.id) && item.status !== "ready").reduce((total, item) => total + item.downloadBytes, 0);
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

  const runInstall = async (component: string, acceptedIdeogramLicense = false, acceptedMuscriptorLicense = false) => {
    setBusy(component);
    setProgress({ component, stage: "preparing", detail: "Checking saved files and available downloads…", downloadedBytes: 0, totalBytes: 0, bytesPerSecond: 0 });
    try {
      const saved = await saveLocations();
      const next = await installSetupComponent({
        component,
        installRoot: saved.settings.installRoot,
        wikipediaEdition: edition,
        acceptIdeogramNonCommercialLicense: acceptedIdeogramLicense,
        whisperCheckpointPath: component === "speech" ? existingModelPaths[WHISPER_MODEL_ID]?.trim() : undefined,
        muscriptorCheckpointPath: component === "muscriptor" ? existingModelPaths[MUSCRIPTOR_MODEL_ID]?.trim() : undefined,
        acceptMuscriptorNonCommercialLicense: acceptedMuscriptorLicense,
        existingModelPaths,
      });
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
      let next = await saveLocations();
      const savedInstallRoot = next.settings.installRoot;
      for (const component of ["assistant", "wikipedia"]) {
        if (next.setup.components.find((item) => item.id === component)?.status === "ready") continue;
        setProgress({ component, stage: "preparing", detail: `Preparing ${component}…`, downloadedBytes: 0, totalBytes: 0, bytesPerSecond: 0 });
        next = await installSetupComponent({ component, installRoot: savedInstallRoot, wikipediaEdition: edition, existingModelPaths });
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

  const install = async (component: string) => {
    if (component === "image") {
      setIdeogramLicenseAccepted(false);
      setIdeogramLicenseOpen(true);
      return;
    }
    if (component === "muscriptor") {
      setMuscriptorLicenseAccepted(false);
      setMuscriptorLicenseOpen(true);
      return;
    }
    await runInstall(component);
  };

  const installProductionSuite = async () => {
    setBusy("production");
    try {
      let next = await saveLocations();
      const savedInstallRoot = next.settings.installRoot;
      for (const component of productionIds) {
        if (next.setup.components.find((item) => item.id === component)?.status === "ready") continue;
        setProgress({ component, stage: "preparing", detail: `Preparing ${component}…`, downloadedBytes: 0, totalBytes: 0, bytesPerSecond: 0 });
        next = await installSetupComponent({ component, installRoot: savedInstallRoot, wikipediaEdition: edition, existingModelPaths });
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

  const openStudio = async (component: "studio" | "music" | "image") => {
    setBusy(`${component}-open`);
    try { await openComfyUi(component); } catch (error) { onError(String(error)); } finally { setBusy(null); }
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

  const chooseModelFile = async (id: string) => {
    try {
      const value = await pickSetupFile("modelAsset");
      if (value) setExistingModelPaths((current) => ({ ...current, [id]: value }));
    } catch (error) { onError(String(error)); }
  };

  const findExistingModels = async () => {
    setScanningModels(true);
    setModelScanMessage("Choosing a folder…");
    try {
      const root = await pickSetupFolder();
      if (!root) { setModelScanMessage(""); return; }
      setModelScanRoot(root);
      setModelScanMessage("Looking for supported model files…");
      const matches = await scanSetupModelFolder(root);
      setExistingModelPaths((current) => ({ ...current, ...matches }));
      const count = Object.keys(matches).length;
      setModelScanMessage(count
        ? `Found ${count} supported model ${count === 1 ? "file" : "files"}. Setup will verify each one before use.`
        : "No release-profile model filenames and sizes matched in this folder. You can still choose renamed files individually below.");
    } catch (error) {
      setModelScanMessage("");
      onError(String(error));
    } finally {
      setScanningModels(false);
    }
  };

  return <div className="setup-console">
    <header className="setup-hero">
      <div className="setup-hero-copy">
        <span className="eyebrow">One-time private setup</span>
        <h1>{snapshot.setup.ready ? productionReady ? "Kestrel is ready." : "Kestrel essentials are ready." : "Let’s make Kestrel work on this computer."}</h1>
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
      <div><span className="eyebrow">Recommended</span><h2>Set up the essentials for me</h2><p>Installs Kestrel's included Ternary Bonsai model, the shared local runtime, and compact offline English Wikipedia. It behaves like every other model you add.</p></div>
      <div className="setup-speed"><label>Internet speed<input type="number" min="1" max="10000" value={speed} onChange={(event) => setSpeed(Math.max(1, Number(event.target.value) || 1))} /><span>Mbps</span></label><small>At {speed} Mbps, essentials need roughly {formatTime(20_980_000_000, speed)} plus verification.</small></div>
      <button className="primary-button setup-main-button" disabled={!!busy} onClick={() => void installEssentials()}>{busy === "essentials" ? <LoaderCircle className="spin" /> : <Download />} Set up essentials</button>
    </section>}

    <section className="setup-location-row">
      <div><strong>Where large AI files live</strong><small>{snapshot.setup.availableBytes ? `${formatBytes(snapshot.setup.availableBytes)} free on the saved drive.` : "Change this before installing if your main drive is small."}</small></div>
      <label><input value={locations.installRoot} onChange={(event) => setLocations((current) => ({ ...current, installRoot: event.target.value }))} /><button onClick={() => void chooseFolder("installRoot")}><FolderOpen /> Browse</button></label>
    </section>

    {!snapshot.setup.gpuName && <div className="setup-hardware-note"><TriangleAlert /><div><strong>No NVIDIA graphics card was detected.</strong><span>The assistant and Wikipedia can still work, although Bonsai may be slow on a laptop processor. Movie Studio stays optional because MiniMax H3 is not practical on this hardware.</span></div></div>}
    {snapshot.setup.gpuName && <div className="setup-hardware-note compatible"><Check /><div><strong>{snapshot.setup.gpuName}</strong><span>{formatBytes(snapshot.setup.gpuMemoryBytes)} graphics memory detected. Kestrel can install the validated NVIDIA profiles for Bonsai, H3, Music 3, Ideogram 4, narration, and dictation.</span></div></div>}

    <section className={`setup-simple-panel setup-production-panel ${productionReady ? "ready" : ""}`}>
      <div><span className="eyebrow">Complete production suite</span><h2>{productionReady ? "Every distributable production service is ready." : "Set up the full studio for me"}</h2><p>Installs movie finishing, H3 video and image generation, Music 3, Chatterbox narration, and timestamped Whisper dictation. Downloads are pinned, resumable, and verified before use.</p><small>MuScriptor audio-to-MIDI remains a separately licensed non-commercial extension; its dedicated card below guides the required producer acceptance and checkpoint import.</small></div>
      {!productionReady && <div className="setup-speed"><strong>{productionBytes ? `${formatBytes(productionBytes)} remaining` : "Models already present"}</strong><small>{productionBytes ? `Roughly ${formatTime(productionBytes, speed)} at ${speed} Mbps, plus extraction and verification.` : "Setup will verify them and add only missing support files."}</small></div>}
      <button className={productionReady ? "quiet-button" : "primary-button setup-main-button"} disabled={!!busy || productionReady} onClick={() => void installProductionSuite()}>{busy === "production" ? <LoaderCircle className="spin" /> : productionReady ? <Check /> : <Download />} {productionReady ? "Production ready" : "Set up production suite"}</button>
    </section>

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
      {components.map((component) => {
        const sharedComfy = component.id === "studio" || component.id === "music" || component.id === "image";
        const opening = busy === `${component.id}-open`;
        const selectedExisting = snapshot.setup.modelAssets?.some((asset) => asset.component === component.id && !!existingModelPaths[asset.id]?.trim());
        const actionLabel = component.status === "ready" && sharedComfy ? "Open ComfyUI"
          : component.status === "ready" ? "Installed"
            : selectedExisting ? "Verify & use existing"
            : component.id === "speech" ? component.status === "partial" ? "Resume Whisper + voice" : "Install Whisper + voice"
              : component.id === "muscriptor" ? component.status === "partial" ? "Resume MuScriptor setup" : "Prepare MuScriptor"
              : component.status === "partial" ? "Resume" : "Install";
        return <article className={`setup-component ${component.status}`} key={component.id}>
        <div className="setup-component-icon">{component.id === "assistant" ? <MessageSquare /> : component.id === "wikipedia" ? <Library /> : component.id === "studio" ? <Film /> : component.id === "music" ? <Headphones /> : component.id === "image" ? <ImageIcon /> : component.id === "speech" ? <Mic2 /> : component.id === "muscriptor" ? <FileMusic /> : <Settings2 />}</div>
        <div className="setup-component-copy"><div className="setup-component-title"><h2>{component.label}</h2><span className={`setup-state ${component.status}`}>{component.status === "ready" ? <><Check /> Ready</> : component.status === "partial" ? "Resume available" : component.optional ? "Optional" : "Needed"}</span></div><p>{component.detail}</p><small>{component.status === "ready" ? component.path : component.downloadBytes ? `${formatBytes(component.downloadBytes)} download · about ${formatTime(component.downloadBytes, speed)} at ${speed} Mbps` : "Model files recognized · only verification or small setup files remain"}</small>
          {component.id === "wikipedia" && component.status !== "ready" && <div className="wikipedia-choice"><button className={edition === "compact" ? "active" : ""} onClick={() => setEdition("compact")}><strong>Compact</strong><span>11.7 GB · article summaries</span></button><button className={edition === "complete" ? "active" : ""} onClick={() => setEdition("complete")}><strong>Complete text</strong><span>49.1 GB · full articles</span></button></div>}
          {component.id === "speech" && component.status !== "ready" && <div className="setup-existing-model"><strong>Whisper is included in this Install button.</strong><span>Already have the official OpenAI-format <code>large-v3-turbo.pt</code>? Choose it here and Kestrel will verify and reuse it instead of downloading another 1.6 GB copy.</span><div className="setup-existing-model-field"><input aria-label="Existing Whisper large-v3-turbo checkpoint" value={existingModelPaths[WHISPER_MODEL_ID] ?? ""} onChange={(event) => setExistingModelPaths((current) => ({ ...current, [WHISPER_MODEL_ID]: event.target.value }))} placeholder="Optional existing large-v3-turbo.pt" /><button type="button" disabled={!!busy} onClick={() => void chooseModelFile(WHISPER_MODEL_ID)}><FolderOpen /> Choose</button></div></div>}
          {component.id === "muscriptor" && component.status !== "ready" && <div className="setup-existing-model muscriptor"><strong>One gated model download, then Kestrel handles the technical setup.</strong><span>1. <a href="https://huggingface.co/MuScriptor/muscriptor-large" target="_blank" rel="noreferrer">Open the official access page</a>, accept its separate non-commercial terms, and download the 5.1 GiB <code>model.safetensors</code>. 2. Wait until the browser download is complete. 3. Choose that file below. Kestrel then prepares roughly 3.3 GiB of isolated Windows CUDA dependencies and proves they work offline.</span><div className="setup-existing-model-field"><input aria-label="Existing MuScriptor large checkpoint" value={existingModelPaths[MUSCRIPTOR_MODEL_ID] ?? ""} onChange={(event) => setExistingModelPaths((current) => ({ ...current, [MUSCRIPTOR_MODEL_ID]: event.target.value }))} placeholder="Completed MuScriptor large model.safetensors" /><button type="button" disabled={!!busy} onClick={() => void chooseModelFile(MUSCRIPTOR_MODEL_ID)}><FolderOpen /> Choose</button></div></div>}
        </div>
        <button className={component.status === "ready" ? "quiet-button" : "primary-button"} disabled={!!busy || (component.status === "ready" && !sharedComfy)} onClick={() => component.status === "ready" && sharedComfy ? void openStudio(component.id as "studio" | "music" | "image") : void install(component.id)}>{busy === component.id || opening ? <LoaderCircle className="spin" /> : component.status === "partial" ? <RefreshCw /> : component.status === "ready" && component.id === "studio" ? <Film /> : component.status === "ready" && component.id === "music" ? <Headphones /> : component.status === "ready" && component.id === "image" ? <ImageIcon /> : <Download />}{actionLabel}</button>
      </article>})}
    </section>

    {progress && <section className="setup-progress" role="status" aria-live="polite">
      <div><LoaderCircle className="spin" /><span><strong>{progress.detail}</strong><small>{progress.totalBytes ? `${formatBytes(progress.downloadedBytes)} of ${formatBytes(progress.totalBytes)}${progress.bytesPerSecond ? ` · ${formatBytes(progress.bytesPerSecond)}/s` : ""}` : "Kestrel will keep completed and partial files."}</small></span></div>
      {progress.totalBytes > 0 && <div className="setup-progress-track"><span style={{ width: `${Math.min(100, progress.downloadedBytes / progress.totalBytes * 100)}%` }} /></div>}
      <button onClick={() => void cancelSetupInstall()}><CircleStop /> Pause safely</button>
    </section>}

    {ideogramLicenseOpen && <dialog ref={ideogramLicenseDialogRef} className="setup-license-dialog" aria-labelledby="ideogram-license-title" onCancel={() => setIdeogramLicenseOpen(false)}><div className="setup-license-icon"><ImageIcon /></div><div><span className="eyebrow">Separate model terms</span><h2 id="ideogram-license-title">Ideogram 4 is non-commercial.</h2><p>The published agreement does not permit client deliverables, promotion, advertising, or other revenue-generating use without separate rights from Ideogram. Kestrel’s MIT license does not change those model terms.</p><a href="https://github.com/ideogram-oss/ideogram4/blob/main/model_licenses/LICENSE-IDEOGRAM-4-NON-COMMERCIAL" target="_blank" rel="noreferrer">Read the complete Ideogram 4 agreement</a><label><input type="checkbox" checked={ideogramLicenseAccepted} onChange={(event) => setIdeogramLicenseAccepted(event.target.checked)} /> I have read and accept the Ideogram Non-Commercial Model Agreement for this installation.</label></div><footer><button disabled={!!busy} onClick={() => setIdeogramLicenseOpen(false)}>Cancel</button><button className="primary-button" disabled={!!busy || !ideogramLicenseAccepted} onClick={() => { setIdeogramLicenseOpen(false); void runInstall("image", true); }}><Download /> Accept and install</button></footer></dialog>}

    {muscriptorLicenseOpen && <dialog ref={muscriptorLicenseDialogRef} className="setup-license-dialog" aria-labelledby="muscriptor-license-title" onCancel={() => setMuscriptorLicenseOpen(false)}><div className="setup-license-icon"><FileMusic /></div><div><span className="eyebrow">Separate gated model terms</span><h2 id="muscriptor-license-title">MuScriptor is for permitted non-commercial transcription.</h2><p>The official weights use CC BY-NC 4.0 plus gated conditions requiring you to have the necessary rights to music you transcribe. Kestrel can prepare the isolated Windows GPU runner, but it cannot accept those terms or grant commercial rights for you.</p><a href="https://huggingface.co/MuScriptor/muscriptor-large" target="_blank" rel="noreferrer">Read and accept the official MuScriptor conditions</a><label><input type="checkbox" checked={muscriptorLicenseAccepted} onChange={(event) => setMuscriptorLicenseAccepted(event.target.checked)} /> I accepted the official conditions, have rights to the music I will transcribe, and understand this extension is non-commercial.</label></div><footer><button disabled={!!busy} onClick={() => setMuscriptorLicenseOpen(false)}>Cancel</button><button className="primary-button" disabled={!!busy || !muscriptorLicenseAccepted || !existingModelPaths[MUSCRIPTOR_MODEL_ID]?.trim()} onClick={() => { setMuscriptorLicenseOpen(false); void runInstall("muscriptor", false, true); }}><Download /> Prepare offline MuScriptor</button></footer></dialog>}

    <button className="setup-advanced-toggle" onClick={() => setAdvanced((value) => !value)}><Settings2 /> Use existing files or choose every location <ChevronDown className={advanced ? "open" : ""} /></button>
    {advanced && <section className="setup-advanced-panel">
      <div className="setup-advanced-heading"><div><span className="eyebrow">Advanced and portable</span><h2>Use files already on this PC</h2><p>Nothing is tied to a drive letter. These saved locations remain editable in the installed app.</p></div><TriangleAlert /></div>
      <section className="setup-model-reuse" aria-labelledby="setup-model-reuse-title">
        <div className="setup-model-reuse-heading"><div><h3 id="setup-model-reuse-title">Reuse every supported model you already have</h3><p>Choose each AI folder once. Kestrel finds known Bonsai, H3, Music 3, Ideogram 4, Chatterbox, Whisper, and MuScriptor files recursively. Scan more than one folder when your library spans drives.</p></div><button className="quiet-button" disabled={!!busy || scanningModels} onClick={() => void findExistingModels()}>{scanningModels ? <LoaderCircle className="spin" /> : <FolderOpen />} Find models in a folder</button></div>
        {(modelScanRoot || modelScanMessage) && <div className="setup-model-scan-status" role="status" aria-live="polite"><strong>{modelScanRoot ? `Last scanned: ${modelScanRoot}` : "Existing model search"}</strong><span>{modelScanMessage}</span></div>}
        <div className="setup-model-assets">
          {(snapshot.setup.modelAssets ?? []).map((asset) => {
            const selected = existingModelPaths[asset.id] ?? "";
            const componentLabel = snapshot.setup.components.find((component) => component.id === asset.component)?.label ?? asset.component;
            return <div className={`setup-model-asset ${asset.recognized ? "recognized" : selected.trim() ? "selected" : ""}`} key={asset.id}>
              <span><strong>{asset.label}</strong><small>{componentLabel} · {formatBytes(asset.bytes)}</small></span>
              {asset.recognized
                ? <span className="setup-model-recognized"><Check /> Recognized at {asset.installedPath}</span>
                : <><input aria-label={`Existing ${asset.label}`} value={selected} onChange={(event) => setExistingModelPaths((current) => ({ ...current, [asset.id]: event.target.value }))} placeholder={`Choose existing ${asset.fileName}`} /><button disabled={!!busy || scanningModels} onClick={() => void chooseModelFile(asset.id)}><FolderOpen /> Choose</button></>}
            </div>;
          })}
        </div>
        <small className="setup-model-reuse-note">Setup checks pinned assets by exact size and SHA-256 before use; gated MuScriptor is size/format checked and locally hashed. A same-drive file is hard-linked when Windows permits it, otherwise it is copied safely. Incompatible variants are never silently substituted.</small>
      </section>
      <PathField label="Included model folder" value={locations.bonsaiRoot} onChange={(value) => setLocations((current) => ({ ...current, bonsaiRoot: value }))} onBrowse={() => void chooseFolder("bonsaiRoot")} />
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
