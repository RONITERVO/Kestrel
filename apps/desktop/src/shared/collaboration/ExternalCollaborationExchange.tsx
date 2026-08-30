import { Check, Copy, FileUp, LoaderCircle } from "lucide-react";
import { useState } from "react";
import { MAX_EXTERNAL_COLLABORATION_BYTES } from "./externalCollaboration";

export function ExternalCollaborationExchange<T>({
  title,
  summary,
  disabled = false,
  buildRequest,
  parseResponse,
  onApply,
  applyLabel = "Validate & use editable draft",
}: {
  title: string;
  summary: string;
  disabled?: boolean;
  buildRequest: () => string | Promise<string>;
  parseResponse: (text: string) => T | Promise<T>;
  onApply: (value: T) => void | Promise<void>;
  applyLabel?: string;
}) {
  const [request, setRequest] = useState("");
  const [response, setResponse] = useState("");
  const [status, setStatus] = useState("");
  const [working, setWorking] = useState(false);
  const locked = disabled || working;

  const copyRequest = async () => {
    if (locked) return;
    setWorking(true);
    setStatus("Building a bounded, versioned production request…");
    try {
      const next = await buildRequest();
      setRequest(next);
      try {
        await navigator.clipboard.writeText(next);
        setStatus("Copied. Paste the request into any chat or agent, then bring its JSON response back here.");
      } catch {
        setStatus("The request is ready below. Select and copy it manually if clipboard access is unavailable.");
      }
    } catch (error) {
      setStatus(String(error));
    } finally {
      setWorking(false);
    }
  };

  const readFile = async (file: File | undefined) => {
    if (locked || !file) return;
    if (file.size > MAX_EXTERNAL_COLLABORATION_BYTES) {
      setStatus("That response is larger than the 2 MiB exchange limit.");
      return;
    }
    try {
      setResponse(await file.text());
      setStatus(`Loaded ${file.name}. Validate it before anything enters the editable project.`);
    } catch (error) {
      setStatus(`Kestrel could not read ${file.name}: ${String(error)}`);
    }
  };

  const apply = async () => {
    if (locked || !response.trim()) return;
    setWorking(true);
    setStatus("Validating the response as production data…");
    try {
      const value = await parseResponse(response);
      await onApply(value);
      setStatus("Loaded as an unsaved editable draft. Review it before saving, generating, or approving anything.");
    } catch (error) {
      setStatus(String(error));
    } finally {
      setWorking(false);
    }
  };

  return <details className="external-plan-exchange external-collaboration-exchange" aria-busy={locked}>
    <summary><span><Copy /> {title}</span><small>{summary}</small></summary>
    <div className="external-plan-body">
      <div className="external-plan-step"><span><b>1</b><strong>Copy the complete request</strong><small>Kestrel includes only the visible production text and a strict JSON contract. It makes no network request. Anything you paste elsewhere is governed by that service’s privacy terms.</small></span><button type="button" disabled={locked} onClick={() => void copyRequest()}>{working ? <LoaderCircle className="spin" /> : <Copy />} Copy request</button></div>
      {request && <label>Manual copy fallback<textarea aria-label={`${title} request`} readOnly disabled={locked} value={request} onFocus={(event) => event.currentTarget.select()} /></label>}
      <div className="external-plan-step"><span><b>2</b><strong>Paste the JSON response</strong><small>The response is parsed as data, never executed. Invalid formats, wrong targets, oversized text, and domain-specific validation failures are rejected.</small></span><label className="external-plan-file"><FileUp /> Choose JSON or text<input disabled={locked} aria-label={`Choose ${title} response`} type="file" accept=".json,.txt,application/json,text/plain" onChange={(event) => void readFile(event.target.files?.[0])} /></label></div>
      <div className="external-plan-drop" onDragOver={(event) => { if (!locked) event.preventDefault(); }} onDrop={(event) => { event.preventDefault(); if (!locked) void readFile(event.dataTransfer.files?.[0]); }}>
        <textarea disabled={locked} aria-label={`${title} JSON response`} maxLength={MAX_EXTERNAL_COLLABORATION_BYTES} value={response} onChange={(event) => setResponse(event.target.value)} placeholder="Paste the returned Kestrel JSON here, or drop a .json/.txt file…" />
      </div>
      <div className="external-plan-actions"><span role="status">{status}</span><button type="button" className="accent" disabled={locked || !response.trim()} onClick={() => void apply()}>{working ? <LoaderCircle className="spin" /> : <Check />} {applyLabel}</button></div>
    </div>
  </details>;
}
