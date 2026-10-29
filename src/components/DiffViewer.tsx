import { useMemo, useState } from "react";
import { Diff, Hunk, parseDiff, type FileData, type ViewType } from "react-diff-view";
import { countChanges, fileDisplayPath, languageForPath, tokenizeFile } from "../lib/diff";
import { useSyntaxReady } from "../lib/highlight";
import { useSettingsStore } from "../lib/settings";
import { Icon } from "./icons";

function filePath(file: FileData): string {
  if (file.type === "delete") return file.oldPath;
  if (file.type === "rename") return `${file.oldPath} → ${file.newPath}`;
  return file.newPath;
}

export function DiffViewer({
  diffText,
  viewType,
}: {
  diffText: string;
  viewType: ViewType;
}) {
  const files = parseDiff(diffText);

  if (files.length === 0) {
    return <p className="muted">No changes between these refs.</p>;
  }

  return (
    <div className="diff-files">
      {files.map((file, index) => (
        <DiffFile
          key={`${file.oldRevision}-${file.newRevision}-${index}`}
          file={file}
          viewType={viewType}
        />
      ))}
    </div>
  );
}

function DiffFile({ file, viewType }: { file: FileData; viewType: ViewType }) {
  const { add, del } = countChanges(file);
  const [collapsed, setCollapsed] = useState(false);
  const maxLines = useSettingsStore((s) => s.largeDiffMaxLines);
  const tooLarge = add + del > maxLines;
  const syntaxReady = useSyntaxReady(languageForPath(fileDisplayPath(file)));
  const tokens = useMemo(
    () => (tooLarge ? undefined : tokenizeFile(file)),
    [file, tooLarge, syntaxReady],
  );
  return (
    <div className="diff-file" data-file-path={fileDisplayPath(file)}>
      <div
        className={`diff-file-header diff-file-toggle${collapsed ? " collapsed" : ""}`}
        role="button"
        tabIndex={0}
        aria-expanded={!collapsed}
        title={collapsed ? "Expand file" : "Collapse file"}
        onClick={() => setCollapsed((c) => !c)}
        onKeyDown={(e) => {
          if (e.key !== "Enter" && e.key !== " ") return;
          e.preventDefault();
          setCollapsed((c) => !c);
        }}
      >
        <Icon name="chev" size={12} className="diff-file-chev" />
        <span className="file-path">{filePath(file)}</span>
        <span className="diff-stats">
          <span className="add">+{add}</span>
          <span className="del">−{del}</span>
        </span>
      </div>
      {collapsed ? null : file.isBinary ? (
        <p className="muted binary-note">Binary file not shown.</p>
      ) : tooLarge ? (
        <p className="muted binary-note">
          Diff too large to display ({(add + del).toLocaleString()} changed lines, limit{" "}
          {maxLines.toLocaleString()}).
        </p>
      ) : (
        <Diff viewType={viewType} diffType={file.type} hunks={file.hunks} tokens={tokens}>
          {(hunks) => hunks.map((hunk) => <Hunk key={hunk.content} hunk={hunk} />)}
        </Diff>
      )}
    </div>
  );
}
