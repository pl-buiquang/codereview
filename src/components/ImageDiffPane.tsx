import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import {
  cropRel,
  diffDim,
  hasImages,
  imageAssetRel,
  imageDiffUrl,
  reportCropRel,
  useImageDiffStatus,
  type ImageSlot,
} from "../lib/imageDiff";
import { useAiDisabledReason, useSettingsStore } from "../lib/settings";
import { toast } from "../lib/toast";
import type {
  ImageChange,
  ImageDiffFile,
  ImageDiffManifest,
  ImageDiffReport,
  ImageDiffStatus,
} from "../lib/types";
import { ImageZoom } from "./ImageZoom";

const CHANGE_LABEL: Record<ImageChange, string> = {
  added: "NEW",
  deleted: "DEL",
  modified: "modified",
  renamed: "renamed",
};

export function fileBadge(f: ImageDiffFile): string {
  if (f.skipped) return "skipped";
  if (f.change === "added" || f.change === "deleted") return CHANGE_LABEL[f.change];
  return f.mismatch_px === 0 ? "same" : `${f.diff_pct < 0.1 ? f.diff_pct.toFixed(3) : f.diff_pct.toFixed(1)}%`;
}

const short = (sha: string | null) => (sha ? sha.slice(0, 8) : "?");

/** The file to show first: the asked one, else the first with something to look at. */
export function initialIndex(files: ImageDiffFile[], path: string | null): number {
  const asked = path != null ? files.findIndex((f) => f.path === path) : -1;
  if (asked >= 0) return asked;
  const changed = files.findIndex((f) => !f.skipped && (f.change !== "modified" || f.mismatch_px > 0));
  return Math.max(0, changed);
}

function Picture({
  reviewId,
  rel,
  fullRel,
  version,
  alt,
  onZoom,
}: {
  reviewId: number;
  rel: string;
  fullRel: string;
  version: string | null;
  alt: string;
  onZoom: (src: string) => void;
}) {
  return (
    <img
      className="image-diff-img"
      loading="lazy"
      alt={alt}
      src={imageDiffUrl(reviewId, rel, version)}
      onClick={() => onZoom(imageDiffUrl(reviewId, fullRel, version))}
    />
  );
}

function RegionRow({
  reviewId,
  file,
  regionId,
  version,
  onZoom,
}: {
  reviewId: number;
  file: ImageDiffFile;
  regionId: number;
  version: string | null;
  onZoom: (src: string) => void;
}) {
  const r = file.regions.find((x) => x.id === regionId);
  if (!r) return null;
  return (
    <div className="image-diff-region">
      <div className="muted image-diff-region-label">
        #{r.id} · {r.w}×{r.h} at {r.x},{r.y} · {r.changed_px.toLocaleString()} px
      </div>
      <div className="image-diff-crops">
        {(["before", "after", "diff"] as ImageSlot[]).map((slot) => {
          const src = imageDiffUrl(reviewId, cropRel(file, r.id, slot), version);
          return <img key={slot} className="image-diff-crop" loading="lazy" alt={`${slot} #${r.id}`} src={src} onClick={() => onZoom(src)} />;
        })}
      </div>
    </div>
  );
}

function FileDetail({
  reviewId,
  file,
  manifest,
  onZoom,
}: {
  reviewId: number;
  file: ImageDiffFile;
  manifest: ImageDiffManifest;
  onZoom: (src: string) => void;
}) {
  const [showRegions, setShowRegions] = useState(true);
  const version = manifest.generated_at;
  const dim = diffDim(file);
  const slots: ImageSlot[] = file.change === "added" ? ["after"] : file.change === "deleted" ? ["before"] : ["before", "after", "diff"];
  const size = (d: { width: number; height: number } | null) => (d ? `${d.width}×${d.height}` : "—");
  return (
    <div className="image-diff-file">
      <div className="image-diff-file-head">
        <span className="file-path" title={file.path}>
          {file.old_path ? `${file.old_path} → ` : ""}
          {file.path}
        </span>
        <span className="image-diff-chip">{CHANGE_LABEL[file.change]}</span>
        {!file.skipped && dim && (
          <span className="muted">
            {size(file.before)} → {size(file.after)} · {file.mismatch_px.toLocaleString()} px differ ({file.diff_pct.toFixed(3)}%)
            {file.aa_px > 0 && ` · ${file.aa_px.toLocaleString()} px anti-aliasing`}
          </span>
        )}
        {file.regions.length > 0 && (
          <label className="image-diff-toggle">
            <input type="checkbox" checked={showRegions} onChange={(e) => setShowRegions(e.target.checked)} /> Outline regions
          </label>
        )}
      </div>
      {file.skipped ? (
        <p className="muted binary-note">Not compared: {file.skipped}</p>
      ) : (
        <>
          <div className="image-diff-cols" style={{ gridTemplateColumns: `repeat(${slots.length}, minmax(0, 1fr))` }}>
            {slots.map((slot) => (
              <div key={slot} className="image-diff-col">
                <h4>{slot}</h4>
                <div className="image-diff-frame">
                  <Picture
                    reviewId={reviewId}
                    rel={imageAssetRel(file, slot, manifest.preview_width)}
                    fullRel={imageAssetRel(file, slot, manifest.preview_width, true)}
                    version={version}
                    alt={`${slot} ${file.path}`}
                    onZoom={onZoom}
                  />
                  {slot === "diff" &&
                    showRegions &&
                    dim &&
                    file.regions.map((r) => (
                      <span
                        key={r.id}
                        className="image-diff-box"
                        title={`#${r.id} · ${r.w}×${r.h}`}
                        style={{
                          left: `${(r.x / dim.width) * 100}%`,
                          top: `${(r.y / dim.height) * 100}%`,
                          width: `${(r.w / dim.width) * 100}%`,
                          height: `${(r.h / dim.height) * 100}%`,
                        }}
                      />
                    ))}
                </div>
              </div>
            ))}
          </div>
          {file.regions.length > 0 && (
            <details className="image-diff-regions">
              <summary>
                Regions ({file.regions.length}
                {file.other_regions > 0 ? ` + ${file.other_regions} smaller` : ""}) · before / after / diff crops
              </summary>
              {file.regions.map((r) => (
                <RegionRow key={r.id} reviewId={reviewId} file={file} regionId={r.id} version={version} onZoom={onZoom} />
              ))}
            </details>
          )}
        </>
      )}
    </div>
  );
}

const KIND_LABEL: Record<string, string> = {
  layout: "layout",
  text: "text",
  color: "color",
  content: "content",
  noise: "noise",
};

/** The AI report: a summary, then each finding with its before / after / diff close-ups. */
function ReportView({
  reviewId,
  status,
  onZoom,
  onShowFile,
}: {
  reviewId: number;
  status: ImageDiffStatus;
  onZoom: (src: string) => void;
  onShowFile: (path: string) => void;
}) {
  const reportQuery = useQuery({
    queryKey: ["image-diff-report", reviewId, status.generated_at],
    queryFn: () => api.imageDiffReport(reviewId),
    enabled: status.has_report,
  });
  const report: ImageDiffReport | null = reportQuery.data ?? null;
  if (status.report_running) {
    return (
      <p className="muted quick-view-note">
        <span className="spinner" /> The AI is looking at the images…
      </p>
    );
  }
  if (!status.has_report || !report) {
    return (
      <p className="muted quick-view-note">
        No report yet. “AI report” asks Claude to go through the changed regions and describe each change.
      </p>
    );
  }
  return (
    <div className="image-diff-report">
      {report.summary && <p className="image-diff-summary">{report.summary}</p>}
      {report.findings.length === 0 && <p className="muted">The report lists no changes.</p>}
      {report.findings.map((f) => (
        <section key={f.n} className={`image-diff-finding${f.kind === "noise" ? " noise" : ""}`}>
          <header>
            <strong>{f.title}</strong>
            <span className="image-diff-chip">{KIND_LABEL[f.kind] ?? f.kind}</span>
            <button className="btn btn-sm btn-ghost" title="Show the whole picture" onClick={() => onShowFile(f.file)}>
              {f.file}
            </button>
          </header>
          {f.description && <p>{f.description}</p>}
          <div className="image-diff-crops">
            {(["before", "after", "diff"] as ImageSlot[]).map((slot) => {
              const src = imageDiffUrl(reviewId, reportCropRel(f.n, slot), report.generated_at);
              return (
                <figure key={slot}>
                  <img className="image-diff-crop" loading="lazy" alt={`${slot} ${f.title}`} src={src} onClick={() => onZoom(src)} />
                  <figcaption className="muted">{slot}</figcaption>
                </figure>
              );
            })}
          </div>
        </section>
      ))}
      {report.dropped > 0 && (
        <p className="muted">{report.dropped} finding(s) left out: they named no compared image.</p>
      )}
    </div>
  );
}

function Legend() {
  return (
    <div className="image-diff-legend muted">
      <span><i className="image-diff-swatch" style={{ background: "#ff0000" }} /> changed</span>
      <span><i className="image-diff-swatch" style={{ background: "#ffff00" }} /> anti-aliasing</span>
      <span><i className="image-diff-swatch" style={{ background: "rgba(128,128,128,0.4)" }} /> unchanged (faded)</span>
    </div>
  );
}

function stateNote(status: ImageDiffStatus): string | null {
  if (status.state === "stale") {
    return `Generated for an earlier version of this review (${short(status.base_sha)}…${short(status.head_sha)}). Regenerate to match the current one.`;
  }
  if (status.state === "failed") return `The last run failed: ${status.error ?? "unknown error"}`;
  if (status.state === "interrupted") return "The last run was interrupted (the app quit).";
  return null;
}

/**
 * Full-width slide-out of a review's image diffs: the changed PNG/JPEG/SVG files as before / after /
 * diff, generated in the background by the backend and read from its disk cache.
 */
export function ImageDiffPane({
  reviewId,
  initialFile,
  onClose,
}: {
  reviewId: number;
  initialFile: string | null;
  onClose: () => void;
}) {
  const queryClient = useQueryClient();
  const statusQuery = useImageDiffStatus(reviewId);
  const status = statusQuery.data;
  const showing = hasImages(status?.state);
  const manifestQuery = useQuery({
    queryKey: ["image-diff-manifest", reviewId, status?.generated_at ?? null],
    queryFn: () => api.imageDiffManifest(reviewId),
    enabled: showing,
  });
  const manifest = manifestQuery.data ?? null;
  const [picked, setPicked] = useState<string | null>(initialFile);
  const [zoom, setZoom] = useState<string | null>(null);
  const [tab, setTab] = useState<"images" | "report">("images");
  const aiOff = useAiDisabledReason();
  const chatModel = useSettingsStore((s) => s.chatModel);
  const envQuery = useQuery({ queryKey: ["environment"], queryFn: api.checkEnvironment, staleTime: Infinity });
  const claudeAvailable = envQuery.data ? envQuery.data.claude != null : true;

  useEffect(() => setPicked(initialFile), [initialFile]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      e.preventDefault();
      onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const generate = useMutation({
    mutationFn: () => api.imageDiffGenerate(reviewId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["image-diff-status", reviewId] }),
    onError: (e) => toast.error(`Could not generate image diffs:\n${String(e)}`),
  });
  const runReport = useMutation({
    mutationFn: () => api.imageDiffRunReport(reviewId, chatModel || undefined),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["image-diff-status", reviewId] });
      setTab("report");
    },
    onError: (e) => toast.error(`Could not start the AI report:\n${String(e)}`),
  });
  const cancel = useMutation({
    mutationFn: () => api.imageDiffCancel(reviewId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["image-diff-status", reviewId] }),
  });

  const files = manifest?.files ?? [];
  const selected = useMemo(() => {
    if (files.length === 0) return null;
    const asked = picked != null ? files.findIndex((f) => f.path === picked) : -1;
    return files[asked >= 0 ? asked : initialIndex(files, null)];
  }, [files, picked]);
  const running = status?.state === "running";
  const note = status ? stateNote(status) : null;
  const anyChange = files.some((f) => !f.skipped && f.mismatch_px > 0);
  // The AI report needs the app's AI switch, `claude`, and pixels that really changed.
  const reportBlocked =
    aiOff ??
    (!claudeAvailable
      ? "Claude CLI not found"
      : !anyChange
        ? "No changed pixels to describe"
        : status?.report_running
          ? "The report is being written…"
          : null);

  return (
    <>
      <div className="quick-view-backdrop" onClick={onClose} />
      <aside className="file-view-pane quick-view-expanded image-diff-pane" aria-label="Image diffs">
        <div className="file-view-pane-header">
          <span className="file-path">Image diffs{manifest ? ` (${files.length})` : ""}</span>
          {running && (
            <>
              <span className="muted">
                <span className="spinner" /> Generating… {status.done}/{status.total || "?"}
              </span>
              <button className="btn btn-sm" onClick={() => cancel.mutate()} disabled={cancel.isPending}>
                Cancel
              </button>
            </>
          )}
          {showing && (
            <button
              className="btn btn-sm"
              disabled={reportBlocked != null || runReport.isPending}
              title={reportBlocked ?? "Ask Claude to describe each change, with before / after / diff close-ups"}
              onClick={() => runReport.mutate()}
            >
              {status?.report_running ? (
                <>
                  <span className="spinner" /> Writing report…
                </>
              ) : status?.has_report ? (
                "Regenerate AI report"
              ) : (
                "AI report"
              )}
            </button>
          )}
          {!running && status && status.state !== "none" && (
            <button
              className="btn btn-sm"
              disabled={generate.isPending}
              title="Generate them again for the review's current version"
              onClick={() => generate.mutate()}
            >
              Regenerate
            </button>
          )}
          <button className="btn-icon" title="Close (Esc)" onClick={onClose}>
            ✕
          </button>
        </div>
        {note && <p className={`image-diff-note${status?.state === "failed" ? " error" : ""}`}>{note}</p>}
        {showing && (
          <div className="image-diff-tabs" role="tablist">
            <button role="tab" aria-selected={tab === "images"} className={tab === "images" ? "active" : ""} onClick={() => setTab("images")}>
              Images
            </button>
            <button role="tab" aria-selected={tab === "report"} className={tab === "report" ? "active" : ""} onClick={() => setTab("report")}>
              Report{status?.has_report ? " ✓" : ""}
            </button>
          </div>
        )}
        <div className="quick-view-split">
          {manifest && files.length > 0 && tab === "images" && (
            <nav className="jump-list quick-view-tree image-diff-list" aria-label="Images">
              <div className="jump-list-content">
                {files.map((f) => (
                  <button
                    key={f.index}
                    className={`image-diff-item${f === selected ? " active" : ""}`}
                    onClick={() => setPicked(f.path)}
                    title={f.path}
                  >
                    <span className="image-diff-item-name">{f.path.split("/").pop()}</span>
                    <span className={`image-diff-badge${f.mismatch_px > 0 ? " changed" : ""}`}>{fileBadge(f)}</span>
                  </button>
                ))}
              </div>
            </nav>
          )}
          <div className="file-view-pane-body quick-view-body image-diff-body">
            {statusQuery.isLoading && <p className="muted quick-view-note">Loading…</p>}
            {status && !showing && !running && (
              <div className="image-diff-empty">
                <p className="muted">No image diffs yet.</p>
                <button className="btn btn-primary" disabled={generate.isPending} onClick={() => generate.mutate()}>
                  Generate image diffs
                </button>
              </div>
            )}
            {running && !manifest && <p className="muted quick-view-note">Generating the image diffs in the background…</p>}
            {manifest && files.length === 0 && <p className="muted quick-view-note">This review changes no images.</p>}
            {showing && status && tab === "report" && (
              <ReportView
                reviewId={reviewId}
                status={status}
                onZoom={setZoom}
                onShowFile={(path) => {
                  setPicked(path);
                  setTab("images");
                }}
              />
            )}
            {manifest && selected && tab === "images" && (
              <>
                <Legend />
                <FileDetail key={selected.index} reviewId={reviewId} file={selected} manifest={manifest} onZoom={setZoom} />
              </>
            )}
          </div>
        </div>
      </aside>
      {zoom && <ImageZoom src={zoom} onClose={() => setZoom(null)} />}
    </>
  );
}
