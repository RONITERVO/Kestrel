import {
  Check,
  Code2,
  Cpu,
  Download,
  FileText,
  FolderOpen,
  Gauge,
  Layers3,
  Library,
  LoaderCircle,
  MemoryStick,
  RefreshCw,
  ShieldCheck,
  TriangleAlert,
  Upload,
  Zap,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import {
  applyModelRuntime,
  exportPromptPackText,
  exportSetupProfileText,
  getControlSnapshot,
  getDefaultPromptPackText,
  getPromptPackText,
  getSetupProfileText,
  getSystemSnapshot,
  importPromptPack,
  importSetupProfile,
  importSetupProfileText,
  pickPromptPackFile,
  resetPromptPack,
  savePromptPackText,
  saveControlSettings,
  saveResearchSettings,
} from "../../platform/api";
import type {
  AppSnapshot,
  ControlSettings,
  ControlSnapshot,
  ResearchSettings,
  SystemSnapshot,
  ThinkingLevel,
} from "../../contracts/index";
import { FlowPages } from "../../shared/book/FlowPages";
import { formatMib } from "../../shared/format";
import { PromptPackChooser, PromptPackPromptEditor, usePromptPackEditor } from "./PromptPackVisualEditor";
import { STANDARD_CONTEXT_OPTIONS, STANDARD_OUTPUT_OPTIONS, findProvenHardwareProfile } from "./modelPolicy";
import "./system.css";

type SystemSection = "models" | "research" | "prompts" | "portable";

const SECTIONS: ReadonlyArray<{ id: SystemSection; icon: typeof Cpu; label: string; detail: string }> = [
  { id: "models", icon: Cpu, label: "Model policy", detail: "Default model, engine, context and model overrides" },
  { id: "research", icon: Library, label: "Research policy", detail: "An optional research-only budget and lanes" },
  { id: "prompts", icon: FileText, label: "Prompt pack", detail: "Every app-authored instruction, editable" },
  { id: "portable", icon: ShieldCheck, label: "Portable setup", detail: "Export or import the safe app setup" },
];

/** The System chapter: live telemetry and the section index on the left page, the open form on the right. */
export function SystemConsole({ visible, initialSettings, initialControl, onSaved, onControlSaved, onImported, onError }: { visible: boolean; initialSettings: ResearchSettings; initialControl: ControlSettings; onSaved: (settings: ResearchSettings) => void; onControlSaved: (control: ControlSnapshot) => void; onImported: (snapshot: AppSnapshot) => void; onError: (message: string) => void }) {
  const [system, setSystem] = useState<SystemSnapshot | null>(null);
  const [researchDraft, setResearchDraft] = useState(initialSettings);
  const [controlDraft, setControlDraft] = useState(initialControl);
  const [tab, setTab] = useState<SystemSection>("models");
  const [overrideModelId, setOverrideModelId] = useState(initialControl.selectedModelId ?? "");
  const [busy, setBusy] = useState<"save-models" | "save-research" | "apply" | "export" | "import" | "refresh-profile" | "save-prompts" | "reset-prompts" | "export-prompts" | "import-prompts" | "reload-prompts" | null>(null);
  const [profilePath, setProfilePath] = useState("");
  const [profileText, setProfileText] = useState("");
  const [profileStatus, setProfileStatus] = useState("");
  const [promptText, setPromptText] = useState("");
  const [lastAppliedPromptText, setLastAppliedPromptText] = useState("");
  const [defaultPromptText, setDefaultPromptText] = useState("");
  const [promptView, setPromptView] = useState<"visual" | "raw" | "files">("visual");
  const [promptPath, setPromptPath] = useState("");
  const [promptStatus, setPromptStatus] = useState("");
  const promptEditor = usePromptPackEditor(promptText, lastAppliedPromptText, defaultPromptText);

  const refreshSystem = useCallback(async () => {
    try {
      const next = await getSystemSnapshot();
      setSystem(next);
      setOverrideModelId((current) => current || next.control.selectedModelId || next.models[0]?.id || "");
    } catch (cause) {
      onError(String(cause));
    }
  }, [onError]);

  const refreshPromptText = useCallback(async () => {
    setBusy("reload-prompts");
    try {
      const text = await getPromptPackText();
      setPromptText(text);
      setLastAppliedPromptText(text);
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  }, [onError]);

  useEffect(() => {
    void getSetupProfileText().then(setProfileText).catch((cause) => onError(String(cause)));
    void refreshPromptText();
    void getDefaultPromptPackText().then(setDefaultPromptText).catch(() => { /* per-prompt "reset to default" stays disabled if this fails */ });
  }, [refreshSystem, onError, refreshPromptText]);

  useEffect(() => {
    if (!visible) return;
    void refreshSystem();
    const timer = window.setInterval(() => void refreshSystem(), 2_500);
    return () => window.clearInterval(timer);
  }, [refreshSystem, visible]);

  const updateResearchNumber = (key: keyof ResearchSettings, value: string) => {
    const parsed = Number.parseInt(value, 10);
    if (Number.isFinite(parsed) && parsed > 0) setResearchDraft((current) => ({ ...current, [key]: parsed }));
  };
  const updateControlNumber = (key: "contextWindow" | "maxOutputTokens" | "threads", value: string) => {
    const parsed = Number.parseInt(value, 10);
    if (Number.isFinite(parsed) && parsed > 0) setControlDraft((current) => ({ ...current, [key]: parsed }));
  };
  const modelOverride = controlDraft.modelOverrides.find((item) => item.modelId === overrideModelId);
  const updateOverrideNumber = (key: "contextWindow" | "maxOutputTokens" | "threads", value: string) => {
    const parsed = Number.parseInt(value, 10);
    if (!overrideModelId || !Number.isFinite(parsed) || parsed <= 0) return;
    setControlDraft((current) => {
      const known = current.modelOverrides.find((item) => item.modelId === overrideModelId) ?? { modelId: overrideModelId };
      return { ...current, modelOverrides: [...current.modelOverrides.filter((item) => item.modelId !== overrideModelId), { ...known, [key]: parsed }] };
    });
  };
  const toggleOverride = (enabled: boolean) => {
    if (!overrideModelId) return;
    setControlDraft((current) => ({
      ...current,
      modelOverrides: enabled
        ? [...current.modelOverrides.filter((item) => item.modelId !== overrideModelId), { modelId: overrideModelId, contextWindow: current.contextWindow, maxOutputTokens: current.maxOutputTokens, threads: current.threads, thinkingLevel: current.thinkingLevel }]
        : current.modelOverrides.filter((item) => item.modelId !== overrideModelId),
    }));
  };
  const saveModels = async () => {
    setBusy("save-models");
    try {
      const saved = await saveControlSettings(controlDraft);
      setControlDraft(saved.settings);
      onControlSaved(saved);
      setProfileStatus("App-wide model policy saved. A loaded model keeps its current launch until restarted.");
      await refreshSystem();
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };
  const saveResearch = async () => {
    setBusy("save-research");
    try {
      const saved = await saveResearchSettings(researchDraft);
      setResearchDraft(saved);
      onSaved(saved);
      setSystem((current) => current ? { ...current, settings: saved } : current);
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };
  const apply = async () => {
    if (!window.confirm("Save this app-wide policy and restart the selected local model? Active local-model work will be interrupted.")) return;
    setBusy("apply");
    try {
      const next = await applyModelRuntime(controlDraft);
      setSystem(next);
      setControlDraft(next.control);
      onControlSaved(await getControlSnapshot(false));
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };
  const refreshProfileText = async () => {
    setBusy("refresh-profile");
    try {
      setProfileText(await getSetupProfileText());
      setProfileStatus("Editable JSON refreshed from the current app-wide setup.");
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };
  const exportProfile = async () => {
    setBusy("export");
    try {
      const transfer = await exportSetupProfileText(profileText);
      setProfilePath(transfer.path);
      setProfileStatus(transfer.message);
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };
  const acceptImported = async (next: AppSnapshot, message: string) => {
    setResearchDraft(next.settings);
    setControlDraft(next.control.settings);
    setOverrideModelId(next.control.settings.selectedModelId ?? next.control.models[0]?.id ?? "");
    onImported(next);
    setProfileText(await getSetupProfileText());
    setProfileStatus(message);
    await refreshSystem();
  };
  const importProfilePath = async () => {
    const path = profilePath.trim();
    if (!path || !window.confirm("Import this setup profile? Existing local paths are used only when they validate, and trust grants remain locked.")) return;
    setBusy("import");
    try {
      await acceptImported(await importSetupProfile(path), "Profile imported, local components rescanned, and trust grants left unchanged.");
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };
  const importProfileText = async () => {
    if (!profileText.trim() || !window.confirm("Apply the edited setup JSON? Kestrel validates every value and local path before saving.")) return;
    setBusy("import");
    try {
      await acceptImported(await importSetupProfileText(profileText), "Edited setup JSON validated and applied across Kestrel.");
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };
  const savePrompts = async () => {
    setBusy("save-prompts");
    try { const next = await savePromptPackText(promptText); setPromptText(next); setLastAppliedPromptText(next); setPromptStatus("Validated and applied to future local-model requests. Active requests keep their captured payload."); }
    catch (cause) { onError(String(cause)); } finally { setBusy(null); }
  };
  const resetPrompts = async () => {
    if (!window.confirm("Reset every app-owned prompt to this Kestrel build's defaults?")) return;
    setBusy("reset-prompts");
    try { const next = await resetPromptPack(); setPromptText(next); setLastAppliedPromptText(next); setPromptStatus("Default prompt pack restored."); }
    catch (cause) { onError(String(cause)); } finally { setBusy(null); }
  };
  const exportPrompts = async () => {
    setBusy("export-prompts");
    try { const transfer = await exportPromptPackText(promptText); setPromptPath(transfer.path); setPromptStatus(transfer.message); }
    catch (cause) { onError(String(cause)); } finally { setBusy(null); }
  };
  const importPrompts = async () => {
    if (!promptPath.trim() || !window.confirm("Import and activate this prompt-only pack for future local-model requests?")) return;
    setBusy("import-prompts");
    try { const next = await importPromptPack(promptPath.trim()); setPromptText(next); setLastAppliedPromptText(next); setPromptStatus("Prompt pack validated, imported, and activated."); }
    catch (cause) { onError(String(cause)); } finally { setBusy(null); }
  };
  const gpu = system?.gpu;
  const usedPercent = gpu ? Math.min(100, (gpu.usedMib / gpu.totalMib) * 100) : 0;
  const models = system?.models ?? [];
  const activeModel = system?.managedRuntime.modelName ?? models.find((item) => item.id === controlDraft.selectedModelId)?.name ?? "No model selected";

  return (
    <div className="system-console spread">
      <div className="page page-left system-index-page">
        <header className="system-hero">
          <div><span className="eyebrow">One runtime policy · every local model</span><h1>System</h1><p>Choose app-wide defaults once. Explicit per-model and workspace settings override them without creating a second server or a hidden model-specific control path.</p></div>
          <div className="system-hero-actions"><button className="quiet-button" onClick={() => void refreshSystem()}><RefreshCw size={15} /> Refresh</button></div>
        </header>

        {tab === "prompts" ? <section className="system-prompt-contents" aria-label="Prompt pack contents">
          <PromptPackChooser editor={promptEditor} onChoose={() => setPromptView("visual")}/>
        </section> : <section className="telemetry-grid" aria-label="Live system telemetry">
          <article className="telemetry-card gpu-card">
            <div className="telemetry-title"><Gauge /><span><small>Detected GPU</small><strong>{gpu?.name ?? "GPU telemetry unavailable"}</strong></span></div>
            {gpu && <><div className="vram-number"><strong>{formatMib(gpu.usedMib)}</strong><span>of {formatMib(gpu.totalMib)} used</span></div><div className="vram-track"><span style={{ width: `${usedPercent}%` }} /></div><div className="telemetry-foot"><span>{formatMib(gpu.freeMib)} free</span><span>{gpu.utilizationPercent}% compute</span></div></>}
          </article>
          <article className="telemetry-card"><div className="telemetry-title"><MemoryStick /><span><small>Local model</small><strong>{activeModel}</strong></span></div><p>{system?.managedRuntime.detail ?? "No managed runtime is loaded."}</p></article>
          <article className="telemetry-card"><div className="telemetry-title"><Cpu /><span><small>Effective runtime</small><strong>{(system?.runtime.contextWindow ?? controlDraft.contextWindow).toLocaleString()} context</strong></span></div><div className="runtime-facts"><span>{(system?.runtime.maxOutputTokens ?? controlDraft.maxOutputTokens).toLocaleString()} max output</span><span>1 inference slot</span><span>{controlDraft.modelOverrides.length} model exception{controlDraft.modelOverrides.length === 1 ? "" : "s"}</span></div></article>
        </section>}

        <nav className="system-tabs" aria-label="System settings sections">
          {SECTIONS.map((section) => {
            const Icon = section.icon;
            return <button key={section.id} className={tab === section.id ? "active" : ""} aria-current={tab === section.id ? "true" : undefined} onClick={() => setTab(section.id)}><Icon size={17} /><span><strong>{section.label}</strong><small>{section.detail}</small></span></button>;
          })}
        </nav>
      </div>

      <div className={`page page-right system-console-body system-section-${tab}`}>
      {tab === "models" && (() => {
        const selectedModel = models.find((m) => m.id === controlDraft.selectedModelId);
        const overrideModel = models.find((m) => m.id === overrideModelId);
        const globalProvenProfile = findProvenHardwareProfile(
          system?.provenHardwareProfiles,
          selectedModel?.name ?? selectedModel?.id,
          gpu?.totalMib,
        );
        const overrideProvenProfile = findProvenHardwareProfile(
          system?.provenHardwareProfiles,
          overrideModel?.name ?? overrideModel?.id,
          gpu?.totalMib,
        );

        return (
          <FlowPages label="Model policy pages" resetKey="models" className="settings-panel system-tab-panel" footer={<div className="settings-actions"><span/><button className="quiet-button" disabled={!!busy} onClick={() => void saveModels()}>{busy === "save-models" ? <LoaderCircle className="spin" size={15}/> : <Check size={15}/>} Save app-wide policy</button><button className="primary-button" disabled={!!busy || models.length === 0} onClick={() => void apply()}>{busy === "apply" ? <LoaderCircle className="spin" size={15}/> : <Zap size={15}/>} Save & restart selected model</button></div>}>
            <div className="settings-heading"><div><span className="eyebrow">Fallback everywhere</span><h2>App-wide local model policy</h2><p>Chat, Computer Tasks, Research, and every Studio inherit these values unless their selected model or workspace has an explicit override.</p></div><label className="advanced-toggle"><input type="checkbox" checked={controlDraft.advancedMode} onChange={(event) => setControlDraft((current) => ({ ...current, advancedMode: event.target.checked }))}/><span/><strong>Allow uncapped values</strong></label></div>
              <div className="system-policy-column">
                <label className="wide-field"><span>Default local model</span><select value={controlDraft.selectedModelId ?? ""} onChange={(event) => { const value = event.target.value || undefined; setControlDraft((current) => ({ ...current, selectedModelId: value })); setOverrideModelId(event.target.value); }}><option value="">First available model</option>{models.map((model) => <option key={model.id} value={model.id}>{model.name}</option>)}</select></label>
                <label className="wide-field"><span>llama.cpp engine</span><input value={controlDraft.enginePath} onChange={(event) => setControlDraft((current) => ({ ...current, enginePath: event.target.value }))}/></label>
                <label className="wide-field"><span>Thinking level (Global fallback)</span><select value={controlDraft.thinkingLevel ?? "high"} onChange={(event) => setControlDraft((current) => ({ ...current, thinkingLevel: event.target.value as ThinkingLevel }))}><option value="off">Off (direct response, no thinking)</option><option value="low">Low reasoning</option><option value="medium">Medium reasoning</option><option value="high">High reasoning (default)</option><option value="max">Max reasoning</option></select></label>
                <div className="model-runtime-row">
                  <TokenDropdownSetting
                    label="Context"
                    hint="Global fallback"
                    value={controlDraft.contextWindow}
                    options={STANDARD_CONTEXT_OPTIONS}
                    recommendedValue={globalProvenProfile?.recommendedContextWindow}
                    recommendedLabel={gpu ? `${formatMib(gpu.totalMib)} GPU` : undefined}
                    disabled={false}
                    allowCustom={controlDraft.advancedMode}
                    onChange={(value) => updateControlNumber("contextWindow", value)}
                  />
                  <TokenDropdownSetting
                    label="Max output"
                    hint="Global fallback"
                    value={controlDraft.maxOutputTokens}
                    options={STANDARD_OUTPUT_OPTIONS}
                    recommendedValue={globalProvenProfile?.recommendedMaxOutputTokens}
                    disabled={false}
                    allowCustom={controlDraft.advancedMode}
                    onChange={(value) => updateControlNumber("maxOutputTokens", value)}
                  />
                  <NumberSetting label="CPU threads" hint="Global fallback" value={controlDraft.threads} disabled={false} onChange={(value) => updateControlNumber("threads", value)}/>
                </div>
              </div>
              <div className="system-policy-column model-exception-card">
                <div className="model-exception-heading"><div><span className="eyebrow">More specific wins</span><strong>Per-model exception</strong></div><label><input type="checkbox" checked={!!modelOverride} disabled={!overrideModelId} onChange={(event) => toggleOverride(event.target.checked)}/> Override this model</label></div>
                <label className="wide-field"><span>Model</span><select value={overrideModelId} onChange={(event) => setOverrideModelId(event.target.value)}><option value="">Choose a model</option>{models.map((model) => <option key={model.id} value={model.id}>{model.name}</option>)}</select></label>
                {overrideProvenProfile && modelOverride && (
                  <div className="proven-profile-banner">
                    <div>
                      <strong><Zap size={14} /> {overrideProvenProfile.displayName}</strong>
                      <p>{overrideProvenProfile.provenSpeedNotes}</p>
                    </div>
                    <button
                      type="button"
                      className="quiet-button compact"
                      disabled={!modelOverride}
                      onClick={() => {
                        if (!overrideModelId) return;
                        setControlDraft((current) => {
                          const known = current.modelOverrides.find((item) => item.modelId === overrideModelId) ?? { modelId: overrideModelId };
                          return {
                            ...current,
                            modelOverrides: [
                              ...current.modelOverrides.filter((item) => item.modelId !== overrideModelId),
                              {
                                ...known,
                                contextWindow: overrideProvenProfile.recommendedContextWindow,
                                maxOutputTokens: overrideProvenProfile.recommendedMaxOutputTokens,
                                thinkingLevel: overrideProvenProfile.recommendedThinkingLevel,
                                threads: overrideProvenProfile.recommendedThreads,
                              },
                            ],
                          };
                        });
                      }}
                    >
                      Auto-tune for GPU
                    </button>
                  </div>
                )}
                <label className="wide-field"><span>Thinking level (Model exception)</span><select disabled={!modelOverride} value={modelOverride?.thinkingLevel ?? controlDraft.thinkingLevel ?? "high"} onChange={(event) => { const val = event.target.value as ThinkingLevel; if (!overrideModelId) return; setControlDraft((current) => { const known = current.modelOverrides.find((item) => item.modelId === overrideModelId) ?? { modelId: overrideModelId }; return { ...current, modelOverrides: [...current.modelOverrides.filter((item) => item.modelId !== overrideModelId), { ...known, thinkingLevel: val }] }; }); }}><option value="off">Off (direct response, no thinking)</option><option value="low">Low reasoning</option><option value="medium">Medium reasoning</option><option value="high">High reasoning</option><option value="max">Max reasoning</option></select></label>
                <div className="model-runtime-row">
                  <TokenDropdownSetting
                    label="Context"
                    hint="Model only"
                    value={modelOverride?.contextWindow ?? controlDraft.contextWindow}
                    options={STANDARD_CONTEXT_OPTIONS}
                    recommendedValue={overrideProvenProfile?.recommendedContextWindow}
                    recommendedLabel={gpu ? `${formatMib(gpu.totalMib)} GPU` : undefined}
                    disabled={!modelOverride}
                    allowCustom={controlDraft.advancedMode}
                    onChange={(value) => updateOverrideNumber("contextWindow", value)}
                  />
                  <TokenDropdownSetting
                    label="Max output"
                    hint="Model only"
                    value={modelOverride?.maxOutputTokens ?? controlDraft.maxOutputTokens}
                    options={STANDARD_OUTPUT_OPTIONS}
                    recommendedValue={overrideProvenProfile?.recommendedMaxOutputTokens}
                    disabled={!modelOverride}
                    allowCustom={controlDraft.advancedMode}
                    onChange={(value) => updateOverrideNumber("maxOutputTokens", value)}
                  />
                  <NumberSetting label="CPU threads" hint="Model only" value={modelOverride?.threads ?? controlDraft.threads} disabled={!modelOverride} onChange={(value) => updateOverrideNumber("threads", value)}/>
                </div>
              </div>
            <div className="advanced-warning"><TriangleAlert/><div><strong>No GPU model is assumed.</strong><span>Kestrel uses detected telemetry and the values you save. Uncapped or oversized settings can still exceed a model or machine limit.</span></div></div>
          </FlowPages>
        );
      })()}

      {tab === "research" && <FlowPages label="Research policy pages" resetKey="research" className="settings-panel system-tab-panel" footer={<div className="settings-actions"><span/><button className="primary-button" disabled={!!busy} onClick={() => void saveResearch()}>{busy === "save-research" ? <LoaderCircle className="spin" size={15}/> : <Check size={15}/>} Save Research policy</button></div>}>
        <div className="settings-heading"><div><span className="eyebrow">Workspace-specific override</span><h2>Offline Research policy</h2><p>Standard Research inherits the selected model's System policy. Enable this only when research genuinely needs a different context/output budget or deeper orchestration.</p></div><label className="advanced-toggle"><input type="checkbox" checked={researchDraft.advancedMode} onChange={(event) => setResearchDraft((current) => ({ ...current, advancedMode: event.target.checked }))}/><span/><strong>Research override</strong></label></div>
        <div className={`advanced-settings ${researchDraft.advancedMode ? "enabled" : "disabled"}`}>
          <NumberSetting label="Context override" hint="Research only" value={researchDraft.contextWindow} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("contextWindow", value)}/>
          <NumberSetting label="Output override" hint="Research only" value={researchDraft.maxOutputTokens} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("maxOutputTokens", value)}/>
          <NumberSetting label="Research lanes" hint="Distinct planning angles" value={researchDraft.researchLanes} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("researchLanes", value)}/>
          <NumberSetting label="Results per lane" hint="Compact candidate memory" value={researchDraft.resultsPerLane} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("resultsPerLane", value)}/>
          <NumberSetting label="Source target" hint="Wikipedia pages" value={researchDraft.sourceTarget} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("sourceTarget", value)}/>
          <NumberSetting label="Tool turns" hint="Search/read rounds" value={researchDraft.toolTurns} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("toolTurns", value)}/>
          <NumberSetting label="Thinking budget" hint="Per reasoning pass" value={researchDraft.thinkingBudget} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("thinkingBudget", value)}/>
          <NumberSetting label="Source characters" hint="Per opened section" value={researchDraft.maxSourceChars} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("maxSourceChars", value)}/>
        </div>
        <div className="single-context-note"><Zap/><div><strong>One selected model, one inference lease</strong><p>Research searches the local archive concurrently, then coordinates evidence through the same managed runtime used by the rest of Kestrel. It never launches or attaches to a separate model-specific server.</p></div></div>
      </FlowPages>}

      {tab === "prompts" && <section className="settings-panel system-tab-panel system-document-page">
        <div className="settings-heading"><div><span className="eyebrow">Advanced · every local workspace</span><h2>Portable prompt pack</h2><p>One prompt-only JSON document holds every instruction Kestrel writes for chat, Computer Tasks, Research, the Studios and model qualification. Producer text stays in its projects.</p></div></div>
        <p className="prompt-pack-authority"><TriangleAlert/><span><strong>Wording guides models; it grants nothing.</strong> Edits cannot add filesystem, network, rendering or tool access, or bypass schema, path, citation or planning checks. Prompt keys are fixed by this build.</span></p>
        <div className="segmented prompt-view-toggle">
          <button className={promptView === "visual" ? "active" : ""} onClick={() => setPromptView("visual")}><Layers3 size={14}/> Visual editor</button>
          <button className={promptView === "raw" ? "active" : ""} onClick={() => setPromptView("raw")}><Code2 size={14}/> Raw JSON</button>
          <button className={promptView === "files" ? "active" : ""} onClick={() => setPromptView("files")}><FolderOpen size={14}/> Pack files</button>
        </div>
        {promptView === "visual" && <PromptPackPromptEditor editor={promptEditor} disabled={!!busy} onChange={setPromptText}/>}
        {promptView === "raw" && <label className="portable-json"><span>Editable prompt-only JSON</span><textarea value={promptText} disabled={!!busy} onChange={(event) => setPromptText(event.target.value)} spellCheck={false} aria-label="Editable portable prompt pack JSON"/></label>}
        {promptView === "files" && <div className="prompt-pack-files">
          <p>Import a pack someone shared, reload the pack Kestrel is using now, or return to this build’s defaults. <strong>Export prompt-only JSON</strong> below writes the text you are editing to a file.</p>
          <div className="portable-path-row">
            <label className="wide-field"><span>Import prompt pack path</span><input value={promptPath} onChange={(event) => setPromptPath(event.target.value)} placeholder="C:\Users\You\Kestrel Research\prompt-packs\kestrel-prompts.json"/></label>
            <button className="quiet-button" disabled={!!busy} onClick={() => void pickPromptPackFile().then((path) => path && setPromptPath(path)).catch((cause) => onError(String(cause)))}><FolderOpen size={15}/> Choose JSON file</button>
            <button className="quiet-button" disabled={!!busy || !promptPath.trim()} onClick={() => void importPrompts()}><Upload size={15}/> Import & activate</button>
          </div>
          <div className="portable-file-buttons">
            <button className="quiet-button" disabled={!!busy} onClick={() => void refreshPromptText()}><RefreshCw size={15}/> Reload active pack</button>
            <button className="danger-button" disabled={!!busy} onClick={() => void resetPrompts()}><RefreshCw size={15}/> Restore build defaults</button>
          </div>
        </div>}
        <div className="settings-actions">{promptStatus ? <span className="profile-status" role="status">{promptStatus}</span> : <span/>}<button className="quiet-button" disabled={!!busy || !promptText.trim()} onClick={() => void savePrompts()}>{busy === "save-prompts" ? <LoaderCircle className="spin" size={15}/> : <Check size={15}/>} Validate & apply</button><button className="primary-button" disabled={!!busy || !promptText.trim()} onClick={() => void exportPrompts()}>{busy === "export-prompts" ? <LoaderCircle className="spin" size={15}/> : <Download size={15}/>} Export prompt-only JSON</button></div>
      </section>}

      {tab === "portable" && <section className="settings-panel system-tab-panel system-document-page">
        <div className="settings-heading"><div><span className="eyebrow">Entire safe app setup</span><h2>Portable setup JSON</h2><p>This editable document covers component locations, archive settings, global model policy, per-model exceptions, Research policy, and every discovered model identity. It intentionally excludes weights, projects, conversations, developer paths, credentials, and access grants.</p></div><button className="quiet-button compact" disabled={!!busy} onClick={() => void refreshProfileText()}>{busy === "refresh-profile" ? <LoaderCircle className="spin" size={15}/> : <RefreshCw size={15}/>} Refresh text from app</button></div>
        <label className="portable-json"><span>Editable profile text</span><textarea value={profileText} onChange={(event) => setProfileText(event.target.value)} spellCheck={false} aria-label="Editable portable setup JSON"/></label>
        <div className="portable-path-row">
          <label className="wide-field"><span>Import an existing profile path</span><input value={profilePath} onChange={(event) => setProfilePath(event.target.value)} placeholder="C:\Users\You\Kestrel Research\setup-profiles\kestrel-profile.json"/></label>
          <button className="quiet-button" disabled={!!busy || !profilePath.trim()} onClick={() => void importProfilePath()}><Upload size={15}/> Import file</button>
        </div>
        <div className="settings-actions">{profileStatus ? <span className="profile-status" role="status">{profileStatus}</span> : <span/>}<button className="quiet-button" disabled={!!busy || !profileText.trim()} onClick={() => void importProfileText()}>{busy === "import" ? <LoaderCircle className="spin" size={15}/> : <Check size={15}/>} Validate & apply edited text</button><button className="primary-button" disabled={!!busy || !profileText.trim()} onClick={() => void exportProfile()}>{busy === "export" ? <LoaderCircle className="spin" size={15}/> : <Download size={15}/>} Export edited JSON</button></div>
      </section>}
      </div>
    </div>
  );
}

export function NumberSetting({ label, hint, value, disabled, onChange }: { label: string; hint: string; value: number; disabled: boolean; onChange: (value: string) => void }) {
  return <label className="number-setting"><span>{label}<small>{hint}</small></span><input type="number" step="1" value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)} /></label>;
}

export function TokenDropdownSetting({
  label,
  hint,
  value,
  options,
  recommendedValue,
  recommendedLabel,
  disabled,
  allowCustom,
  onChange,
}: {
  label: string;
  hint: string;
  value: number;
  options: Array<{ value: number; label: string }>;
  recommendedValue?: number;
  recommendedLabel?: string;
  disabled: boolean;
  allowCustom?: boolean;
  onChange: (value: string) => void;
}) {
  const isKnown = options.some((opt) => opt.value === value);
  const [customMode, setCustomMode] = useState(() => !isKnown);

  return (
    <label className="number-setting token-tier-setting">
      <span>
        {label}
        <small>{hint}</small>
      </span>
      {!customMode ? (
        <select
          value={isKnown ? value : "custom"}
          disabled={disabled}
          onChange={(event) => {
            if (event.target.value === "custom") {
              setCustomMode(true);
            } else {
              onChange(event.target.value);
            }
          }}
        >
          {options.map((opt) => {
            const isRec = recommendedValue === opt.value;
            const text = isRec
              ? `${opt.label} · Recommended ${recommendedLabel ? `(${recommendedLabel})` : ""}`
              : opt.label;
            return (
              <option key={opt.value} value={opt.value}>
                {text}
              </option>
            );
          })}
          {allowCustom && <option value="custom">Custom...</option>}
        </select>
      ) : (
        <div className="token-custom-entry">
          <input
            type="number"
            step="1024"
            value={value}
            disabled={disabled}
            onChange={(event) => onChange(event.target.value)}
          />
          <button
            type="button"
            className="quiet-button compact"
            onClick={() => setCustomMode(false)}
            title="Switch back to presets"
          >
            Presets
          </button>
        </div>
      )}
    </label>
  );
}

