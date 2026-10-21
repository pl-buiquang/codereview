# Import and export

## Export

Open the review actions menu and choose Export. The modal previews the Markdown (AI-readable) and offers:

- **Save Markdown** / **Save JSON**: asks for a destination, writes the file, copies its path to the clipboard.
- **Copy Markdown** / **Copy JSON**: puts the rendered export on the clipboard; no file is written.
- **Export to VSCode**: writes `.vscode/reviews/r{id}.json` inside the repository's local checkout, in the format of the VSCode Code Review plugin, and copies the path. Shown only for reviews whose repository has a local clone; it fails if the path is missing on disk.

Exporting is repeatable and never locks the review or changes its status. Saving a file or exporting to VSCode stamps `last_exported_at`; copying to the clipboard does not. Whole-file and file-view comments are included, and replies are folded under their root comment.

## Import

Click **Import** in the Reviews list header and pick a JSON file. A new **draft** review is created from it. The format matches the JSON export, so you can round-trip: export, edit, re-import. You can also write a review JSON from scratch, for example to feed in AI-generated reviews.

Each import creates a new target and a new review; it does not merge into an existing one. Unknown fields are ignored.

### Minimal example

Only `title` is required:

```json
{
  "title": "Quick note on the parser"
}
```

### Full example

```json
{
  "title": "Improve error handling in parser",
  "repo": "owner/name",
  "kind": "github_pr",
  "github_pr_number": 42,
  "base_ref": "main",
  "head_ref": "feature/parser",
  "base_sha": "abcdef1234567890",
  "head_sha": "1234567890abcdef",
  "verdict": "request_changes",
  "summary": "A few issues to fix before merging.",
  "comments": [
    {
      "file": "src/parser.rs",
      "side": "RIGHT",
      "line": 15,
      "start_line": 12,
      "diff_hunk": "@@ -10,5 +10,8 @@\n fn parse() {\n+    let x = 1;",
      "body": "This should handle the empty-input case.",
      "subject_type": "line",
      "origin": "diff",
      "resolved_at": null,
      "replies": [
        { "body": "Good catch, will fix." }
      ]
    },
    {
      "file": "src/parser.rs",
      "subject_type": "file",
      "body": "Overall the module needs a doc comment."
    }
  ]
}
```

### Field reference

| Field | Type | Default | Description |
|---|---|---|---|
| `title` | string | **required** | Review title (shown in the reviews list) |
| `repo` | string | `"imported"` | `"owner/name"` matches a repository by remote owner and name, otherwise a remote-only repository is created. Any other string is matched against a repository's local path, otherwise a new repository with that path is created. |
| `kind` | `"github_pr"` \| `"local"` | `"local"` | GitHub PR or local branch comparison. Any other value fails the import. |
| `github_pr_number` | number \| null | `null` | PR number (only meaningful when `kind` is `"github_pr"`) |
| `base_ref` | string | `"unknown"` | Base branch/ref name |
| `head_ref` | string | `"unknown"` | Head branch/ref name |
| `base_sha` | string \| null | `null` | Resolved base commit SHA (becomes the review's pin) |
| `head_sha` | string \| null | `null` | Resolved head commit SHA (becomes the review's pin) |
| `verdict` | `"approve"` \| `"request_changes"` \| `"comment"` \| null | `null` | Review verdict. Any other value is treated as no verdict. |
| `summary` | string | `""` | Review body (Markdown) |
| `comments` | array | `[]` | Comments (see below) |

**Comment fields:**

| Field | Type | Default | Description |
|---|---|---|---|
| `file` | string | **required** | File path the comment is anchored to |
| `body` | string | **required** | Comment text (Markdown; supports `` ```suggestion `` fences) |
| `side` | `"LEFT"` \| `"RIGHT"` | `"RIGHT"` | `LEFT` = old/base, `RIGHT` = new/head. Other values are rejected by the database. |
| `line` | number | `0` | Line number the comment points at |
| `start_line` | number \| null | `null` | Start line for multi-line range comments |
| `diff_hunk` | string \| null | `null` | The diff hunk context for the comment |
| `subject_type` | `"line"` \| `"file"` | `"line"` | `"file"` for whole-file comments (line is ignored). Other values are rejected. |
| `origin` | `"diff"` \| `"file_view"` | `"diff"` | Where the comment was authored; `"file_view"` comments fold into the body on publish. Other values are rejected. |
| `resolved_at` | string \| null | `null` | ISO-8601 timestamp if the thread is resolved |
| `replies` | array | `[]` | Thread replies: `[{ "body": "..." }]`. A `created_at` key is accepted but ignored. |

Replies inherit the root comment's file, side, line and origin. All imported comments get the import time as their created time.

Where it lives: `src-tauri/src/commands/export.rs` (commands, `import_review`), `src-tauri/src/export.rs` (rendering), `ExportModal` in `src/components/ReviewView.tsx`, Import button in `src/components/ReviewsView.tsx`.
