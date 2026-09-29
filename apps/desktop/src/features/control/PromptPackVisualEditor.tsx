import { AlertTriangle, RotateCcw, Search, Tag, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { PromptPack } from "../../contracts/index";
import { PagedList } from "../../shared/book/PagedList";
import "./prompt-pack.css";

interface PromptEntry {
  key: string;
  category: string;
  value: string;
}

const MAX_PROMPT_BYTES = 64 * 1024;

function parsePromptsMap(jsonText: string): Record<string, string> | null {
  try {
    const parsed: unknown = JSON.parse(jsonText);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return null;
    const prompts = (parsed as { prompts?: unknown }).prompts;
    if (!prompts || typeof prompts !== "object" || Array.isArray(prompts)) return null;
    const map: Record<string, string> = {};
    for (const [key, value] of Object.entries(prompts as Record<string, unknown>)) {
      if (typeof value === "string") map[key] = value;
    }
    return map;
  } catch {
    return null;
  }
}

function categoryOf(key: string): string {
  const dot = key.indexOf(".");
  return dot === -1 ? key : key.slice(0, dot);
}

// Mirrors prompt_catalog.rs's `placeholders` scan exactly, so client-side hints match
// what native validation will actually accept.
function extractPlaceholders(text: string): Set<string> {
  const found = new Set<string>();
  let rest = text;
  for (;;) {
    const start = rest.indexOf("{{");
    if (start === -1) break;
    rest = rest.slice(start + 2);
    const end = rest.indexOf("}}");
    if (end === -1) break;
    const name = rest.slice(0, end).trim();
    if (name) found.add(name);
    rest = rest.slice(end + 2);
  }
  return found;
}

function sameMembers(a: Set<string>, b: Set<string>): boolean {
  if (a.size !== b.size) return false;
  for (const item of a) if (!b.has(item)) return false;
  return true;
}

function withPromptValue(jsonText: string, key: string, value: string): string {
  const parsed = JSON.parse(jsonText) as PromptPack;
  parsed.prompts = { ...parsed.prompts, [key]: value };
  return JSON.stringify(parsed, null, 2);
}

function previewOf(value: string): string {
  const flat = value.trim().replace(/\s+/g, " ");
  if (!flat) return "(Empty prompt)";
  return flat.length > 140 ? `${flat.slice(0, 140)}…` : flat;
}

/**
 * Shared state for the prompt pack editor. The chooser (search, categories, prompt list) sits on the
 * left page and the open prompt on the right, so both read the same selection.
 */
export function usePromptPackEditor(jsonText: string, savedJsonText: string, defaultJsonText: string) {
  const [search, setSearch] = useState("");
  const [category, setCategory] = useState("ALL");
  const [selectedKey, setSelectedKey] = useState("");

  const currentMap = useMemo(() => parsePromptsMap(jsonText), [jsonText]);
  const savedMap = useMemo(() => parsePromptsMap(savedJsonText) ?? {}, [savedJsonText]);
  const defaultMap = useMemo(() => parsePromptsMap(defaultJsonText) ?? {}, [defaultJsonText]);

  const entries: PromptEntry[] = useMemo(() => {
    if (!currentMap) return [];
    return Object.entries(currentMap).map(([key, value]) => ({ key, value, category: categoryOf(key) }));
  }, [currentMap]);

  const categories = useMemo(() => {
    const counts = new Map<string, number>();
    entries.forEach((entry) => counts.set(entry.category, (counts.get(entry.category) ?? 0) + 1));
    return Array.from(counts.entries()).sort(([a], [b]) => a.localeCompare(b));
  }, [entries]);

  const filtered = useMemo(() => {
    const term = search.trim().toLowerCase();
    return entries.filter((entry) => {
      if (category !== "ALL" && entry.category !== category) return false;
      if (!term) return true;
      return entry.key.toLowerCase().includes(term) || entry.value.toLowerCase().includes(term);
    });
  }, [entries, search, category]);

  useEffect(() => {
    if (filtered.length === 0) return;
    if (!filtered.some((entry) => entry.key === selectedKey)) setSelectedKey(filtered[0].key);
  }, [filtered, selectedKey]);

  return {
    jsonText, valid: currentMap !== null, search, setSearch, category, setCategory, selectedKey, setSelectedKey,
    entries, categories, filtered, savedMap, defaultMap,
  };
}

export type PromptPackEditor = ReturnType<typeof usePromptPackEditor>;

/** Search, category filters and every prompt key, whole rows a page at a time. */
export function PromptPackChooser({ editor, onChoose }: { editor: PromptPackEditor; onChoose?: () => void }) {
  const { entries, categories, filtered, savedMap, search, setSearch, category, setCategory, selectedKey, setSelectedKey } = editor;
  if (!editor.valid) {
    return <p className="prompt-visual-empty-list">Fix the pack in Raw JSON to list its prompts here.</p>;
  }
  return (
    <div className="prompt-visual-chooser">
      <div className="prompt-visual-toolbar">
        <label className="search-field">
          <Search size={15}/>
          <input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="Search prompt keys or text…"/>
          {search && <button aria-label="Clear search" onClick={() => setSearch("")}><X size={13}/></button>}
        </label>
        <div className="prompt-visual-categories">
          <button aria-label={`Show all categories (${entries.length} prompts)`} className={category === "ALL" ? "active" : ""} onClick={() => setCategory("ALL")}>All <span>{entries.length}</span></button>
          {categories.map(([name, count]) => <button key={name} aria-label={`Show ${name} category (${count} prompts)`} className={category === name ? "active" : ""} onClick={() => setCategory(name)}>{name} <span>{count}</span></button>)}
        </div>
      </div>
      <PagedList
        className="prompt-visual-list"
        label="Prompt keys"
        items={filtered}
        itemKey={(entry) => entry.key}
        selectedKey={selectedKey}
        empty={<p className="prompt-visual-empty-list">No prompts match this search.</p>}
        renderItem={(entry) => {
          const modified = savedMap[entry.key] !== undefined && savedMap[entry.key] !== entry.value;
          return (
            <button aria-label={`Edit prompt ${entry.key}`} className={entry.key === selectedKey ? "active" : ""} onClick={() => { setSelectedKey(entry.key); onChoose?.(); }}>
              <span className="prompt-visual-list-key">{entry.key}</span>
              <span className="prompt-visual-list-preview">{previewOf(entry.value)}</span>
              {modified && <span className="prompt-visual-modified-dot" title="Modified since the last saved pack"/>}
            </button>
          );
        }}
      />
    </div>
  );
}

/** The open prompt: its variables, revert and reset, and the text itself. */
export function PromptPackPromptEditor({ editor, disabled, onChange }: { editor: PromptPackEditor; disabled: boolean; onChange: (next: string) => void }) {
  if (!editor.valid) {
    return <div className="prompt-visual-invalid"><AlertTriangle/><div><strong>The JSON has a syntax error.</strong><span>Switch to Raw JSON to fix the syntax, then return to Visual editor.</span></div></div>;
  }
  const selected = editor.entries.find((entry) => entry.key === editor.selectedKey) ?? null;
  const updateValue = (value: string) => {
    if (!selected) return;
    onChange(withPromptValue(editor.jsonText, selected.key, value));
  };

  const savedValue: string | undefined = selected ? editor.savedMap[selected.key] : undefined;
  const defaultValue: string | undefined = selected ? editor.defaultMap[selected.key] : undefined;
  const isModifiedFromSaved = selected !== null && savedValue !== undefined && savedValue !== selected.value;
  const canResetToDefault = selected !== null && defaultValue !== undefined && defaultValue !== selected.value;

  const requiredKnown = defaultValue !== undefined;
  const requiredVariables = requiredKnown ? extractPlaceholders(defaultValue) : new Set<string>();
  const actualVariables = selected ? extractPlaceholders(selected.value) : new Set<string>();
  const variablesMatch = !requiredKnown || sameMembers(requiredVariables, actualVariables);
  const allVariableNames = Array.from(new Set([...requiredVariables, ...actualVariables])).sort();

  const wordCount = selected ? selected.value.split(/\s+/).filter(Boolean).length : 0;
  const byteCount = selected ? new TextEncoder().encode(selected.value).length : 0;

  return (
    <section className="prompt-visual-detail" aria-label="Selected prompt">
      {selected ? <>
        <div className="prompt-visual-detail-head">
          <div><span className="prompt-visual-detail-key">{selected.key}</span><span className="chip note">{selected.category}</span></div>
          <div className="prompt-visual-detail-actions">
            <button className="quiet-button compact" disabled={disabled || !isModifiedFromSaved} onClick={() => savedValue !== undefined && updateValue(savedValue)} title="Revert this prompt to the last saved pack"><RotateCcw size={14}/> Revert to saved</button>
            <button className="quiet-button compact" disabled={disabled || !canResetToDefault} onClick={() => defaultValue !== undefined && updateValue(defaultValue)} title="Reset only this prompt to the build default"><RotateCcw size={14}/> Reset to default</button>
          </div>
        </div>
        <div className="prompt-visual-variables">
          <Tag size={14}/><span>Variables:</span>
          {allVariableNames.length === 0 && <span className="prompt-visual-no-vars">None in this prompt.</span>}
          {!requiredKnown && Array.from(actualVariables).sort().map((name) => <span key={name} className="prompt-visual-var neutral">{"{{"}{name}{"}}"}</span>)}
          {requiredKnown && allVariableNames.map((name) => {
            const isRequired = requiredVariables.has(name);
            const isPresent = actualVariables.has(name);
            const state = isRequired && isPresent ? "ok" : isRequired ? "missing" : "extra";
            return <span key={name} className={`prompt-visual-var ${state}`} title={state === "missing" ? "Required but missing from this prompt" : state === "extra" ? "Not a recognized variable for this prompt" : "Required and present"}>{"{{"}{name}{"}}"}</span>;
          })}
        </div>
        {requiredKnown && !variablesMatch && <div className="prompt-visual-warning"><AlertTriangle size={15}/><span>Kestrel will reject this prompt until it uses exactly these variables: {requiredVariables.size > 0 ? Array.from(requiredVariables).map((name) => `{{${name}}}`).join(", ") : "none"}.</span></div>}
        <textarea className="prompt-visual-textarea" value={selected.value} disabled={disabled} onChange={(event) => updateValue(event.target.value)} spellCheck={false} aria-label={`Prompt text for ${selected.key}`}/>
        <div className="prompt-visual-meta"><span>{wordCount} words · {selected.value.length} chars</span><span className={byteCount > MAX_PROMPT_BYTES ? "prompt-visual-meta-over" : ""}>{(byteCount / 1024).toFixed(1)} / 64 KiB</span></div>
      </> : <p className="prompt-visual-empty-list">Choose a prompt on the left page to edit its text.</p>}
    </section>
  );
}
