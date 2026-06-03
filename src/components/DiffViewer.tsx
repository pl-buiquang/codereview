import { useMemo } from "react";
import { Diff, Hunk, parseDiff, type FileData, type ViewType } from "react-diff-view";
import { tokenizeFile } from "../lib/diff";

function countChanges(file: FileData): { add: number; del: number } {
  let add = 0;
  let del = 0;
  for (const hunk of file.hunks) {
    for (const change of hunk.changes) {
      if (change.type === "insert") add++;
      else if (change.type === "delete") del++;
    }
  }
  return { add, del };
}

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
  const tokens = useMemo(() => tokenizeFile(file), [file]);
  return (
    <div className="diff-file">
      <div className="diff-file-header">
        <span className="file-path">{filePath(file)}</span>
        <span className="diff-stats">
          <span className="add">+{add}</span>
          <span className="del">−{del}</span>
        </span>
      </div>
      {file.isBinary ? (
        <p className="muted binary-note">Binary file not shown.</p>
      ) : (
        <Diff viewType={viewType} diffType={file.type} hunks={file.hunks} tokens={tokens}>
          {(hunks) => hunks.map((hunk) => <Hunk key={hunk.content} hunk={hunk} />)}
        </Diff>
      )}
    </div>
  );
}
