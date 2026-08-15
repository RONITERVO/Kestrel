import {
  Check,
  CircleStop,
  Download,
  LoaderCircle,
  RefreshCw,
  Search,
} from "lucide-react";
import { useEffect, useState } from "react";
import {
  cancelModelDownload,
  getControlSnapshot,
  inspectModelDownload,
  listModelDownloads,
  onModelDownload,
  resumeModelDownload,
  startModelDownload,
} from "./api";
import type {
  ControlSnapshot,
  ModelDownloadInspection,
  ModelDownloadRecord,
} from "./types";

export function ModelDownloader({
  gpuTotalMib,
  onCatalogChanged,
  onError,
  variant = "control",
}: {
  gpuTotalMib?: number;
  onCatalogChanged: (control: ControlSnapshot) => void;
  onError: (message: string) => void;
  variant?: "control" | "setup";
}) {
  const [url, setUrl] = useState("");
  const [sha256, setSha256] = useState("");
  const [inspection, setInspection] = useState<ModelDownloadInspection | null>(null);
  const [candidateUrl, setCandidateUrl] = useState("");
  const [records, setRecords] = useState<ModelDownloadRecord[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const update = (record: ModelDownloadRecord) =>
    setRecords((current) => [
      record,
      ...current.filter((item) => item.id !== record.id),
    ]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listModelDownloads()
      .then((items) => {
        if (!disposed) setRecords(items);
      })
      .catch((cause) => {
        if (!disposed) onError(String(cause));
      });
    void onModelDownload(update).then((next) => {
      if (disposed) next();
      else unlisten = next;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const active = records.find((record) =>
    ["inspecting", "downloading", "retrying", "verifying"].includes(
      record.status,
    ),
  );
  const run = async (operation: () => Promise<ModelDownloadRecord>) => {
    setSubmitting(true);
    try {
      const record = await operation();
      update(record);
      if (record.status === "complete")
        onCatalogChanged(await getControlSnapshot(false));
    } catch (cause) {
      onError(String(cause));
      setRecords(await listModelDownloads().catch(() => records));
    } finally {
      setSubmitting(false);
    }
  };
  const begin = () => {
    const candidate = inspection?.candidates.find((item) => item.url === candidateUrl);
    const source = candidate?.url ?? url.trim();
    if (!source) return;
    void run(() =>
      startModelDownload({
        url: source,
        expectedSha256: sha256.trim() || candidate?.sha256 || undefined,
      }),
    );
  };
  const inspect = async () => {
    if (!url.trim()) return;
    setSubmitting(true);
    try {
      const result = await inspectModelDownload(url.trim());
      setInspection(result);
      setCandidateUrl(
        result.candidates.length === 1 && result.candidates[0].kind !== "model-shard"
          ? result.candidates[0].url
          : "",
      );
    } catch (cause) {
      onError(String(cause));
    } finally {
      setSubmitting(false);
    }
  };
  const recentIds = new Set(records.slice(0, 4).map((record) => record.id));
  const newest = records.filter(
    (record) =>
      recentIds.has(record.id) ||
      ["paused", "interrupted", "failed"].includes(record.status),
  );

  return (
    <section
      className={`model-downloader wide model-downloader-${variant}`}
      aria-label="Observed model downloader"
    >
      <header>
        <span>
          <Download />
          <strong>Observed model downloader</strong>
          <small>Explicit public-network transfer · durable and resumable</small>
        </span>
        {active && <i>{active.status}</i>}
      </header>
      <p>
        Paste a public Hugging Face repository or <code>.gguf</code> file page.
        Kestrel lists bounded GGUF choices, checks publisher size and checksum,
        retries temporary failures, and never starts or resumes by itself.
      </p>
      <label className="model-download-url">
        Hugging Face model repository or GGUF URL
        <input
          type="url"
          value={url}
          disabled={!!active || submitting}
          onChange={(event) => {
            setUrl(event.target.value);
            setInspection(null);
            setCandidateUrl("");
          }}
          placeholder="https://huggingface.co/owner/repository"
        />
      </label>
      {inspection && (
        <div className="model-download-candidates">
          <span>
            <strong>{inspection.repository}</strong>
            <small>{inspection.detail}</small>
          </span>
          {inspection.candidates.length > 0 && (
            <label>
              GGUF file
              <select
                value={candidateUrl}
                disabled={!!active || submitting}
                onChange={(event) => setCandidateUrl(event.target.value)}
              >
                <option value="">Choose a quantization…</option>
                {inspection.candidates.map((candidate) => (
                  <option
                    key={candidate.url}
                    value={candidate.url}
                    disabled={candidate.kind === "model-shard"}
                  >
                    {candidate.filePath} · {formatBytes(candidate.bytes)}
                    {candidate.kind === "model-shard"
                      ? " · requires grouped shard support"
                      : candidate.kind !== "model"
                        ? ` · ${candidate.kind}`
                        : ""}
                  </option>
                ))}
              </select>
            </label>
          )}
        </div>
      )}
      <details>
        <summary>Optional publisher checksum</summary>
        <label>
          Expected SHA-256
          <input
            value={sha256}
            disabled={!!active || submitting}
            onChange={(event) => setSha256(event.target.value)}
            placeholder="64 hexadecimal characters"
          />
        </label>
        <small>
          If Hugging Face exposes an LFS SHA-256, Kestrel adopts it automatically.
          A value entered here must agree with that metadata.
        </small>
      </details>
      <div className="model-download-actions">
        <button
          type="button"
          disabled={!!active || submitting || !url.trim()}
          onClick={() => void inspect()}
        >
          {submitting && !active ? <LoaderCircle className="spin" /> : <Search />}
          Inspect choices
        </button>
        <button
          type="button"
          disabled={
            !!active ||
            submitting ||
            (!candidateUrl && !/\.gguf(?:[?#]|$)/i.test(url.trim()))
          }
          onClick={begin}
        >
          {submitting && !active ? (
            <LoaderCircle className="spin" />
          ) : (
            <Download />
          )}
          Start observed transfer
        </button>
        {active && (
          <button
            type="button"
            className="stop"
            onClick={() => void cancelModelDownload()}
          >
            <CircleStop /> Stop safely
          </button>
        )}
      </div>
      {active && <ModelDownloadProgress record={active} gpuTotalMib={gpuTotalMib} />}
      {newest.length > 0 && (
        <div className="model-download-history">
          {newest.map((record) => {
            const resumable =
              ["paused", "interrupted", "failed"].includes(record.status) &&
              (record.totalBytes === 0 ||
                record.downloadedBytes <= record.totalBytes);
            return (
              <article key={record.id}>
                <span>
                  <strong>{record.fileName}</strong>
                  <small>
                    {record.status} · {formatBytes(record.downloadedBytes)}
                    {record.totalBytes
                      ? ` of ${formatBytes(record.totalBytes)}`
                      : ""}
                  </small>
                  <em title={record.detail}>{record.detail}</em>
                </span>
                {resumable && !active && (
                  <button
                    type="button"
                    disabled={submitting}
                    onClick={() => void run(() => resumeModelDownload(record.id))}
                  >
                    <RefreshCw /> {record.status === "failed" ? "Retry" : "Resume"}
                  </button>
                )}
                {record.status === "complete" && <Check />}
              </article>
            );
          })}
        </div>
      )}
      <footer>
        During an active transfer Kestrel keeps Windows awake while allowing the
        display to turn off. Shutdown, loss of network, or closing Kestrel
        preserves the partial file; continuation always requires an explicit
        Resume.
      </footer>
    </section>
  );
}

function ModelDownloadProgress({
  record,
  gpuTotalMib,
}: {
  record: ModelDownloadRecord;
  gpuTotalMib?: number;
}) {
  const percent = record.totalBytes
    ? Math.min(100, (record.downloadedBytes / record.totalBytes) * 100)
    : 0;
  const modelMib = record.totalBytes / 1024 / 1024;
  const fit = gpuTotalMib
    ? modelMib + 2048 <= gpuTotalMib
      ? "File size leaves at least 2 GiB of nominal VRAM headroom; context still decides final fit."
      : "File size alone leaves less than 2 GiB of nominal VRAM headroom; full-GPU startup is unlikely."
    : "GPU fit will be evaluated separately at runtime; download size is not a VRAM guarantee.";
  return (
    <div className="model-download-progress" role="status" aria-live="polite">
      <div>
        <span>
          <strong>{record.status}</strong>
          <small>{record.detail}</small>
        </span>
        <b>{percent.toFixed(1)}%</b>
      </div>
      <div className="model-download-track">
        <i style={{ width: `${percent}%` }} />
      </div>
      <dl>
        <div>
          <dt>Received</dt>
          <dd>{formatBytes(record.downloadedBytes)}</dd>
        </div>
        <div>
          <dt>Speed</dt>
          <dd>
            {record.bytesPerSecond
              ? `${formatBytes(record.bytesPerSecond)}/s`
              : "—"}
          </dd>
        </div>
        <div>
          <dt>ETA</dt>
          <dd>{formatDuration(record.etaSeconds)}</dd>
        </div>
        <div>
          <dt>Retries</dt>
          <dd>{record.retryCount}</dd>
        </div>
      </dl>
      <small className="model-fit-guidance">{fit}</small>
    </div>
  );
}

function formatDuration(seconds?: number) {
  if (seconds === undefined) return "—";
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600)
    return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
}

function formatBytes(value: number) {
  if (value >= 1024 * 1024 * 1024)
    return `${(value / 1024 / 1024 / 1024).toFixed(2)} GiB`;
  if (value >= 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MiB`;
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${value} B`;
}
