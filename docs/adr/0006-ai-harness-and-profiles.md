# All AI calls go through one `Harness` trait; profiles select the harness

Every AI feature used to call the `claude` CLI directly, from about six places: the review chat, theme suggestions, session summaries (extract and fork), the routine manager, and three builders of interactive-session argv. Each had its own flag handling, so supporting a second CLI would have meant touching all of them.

`ai::Harness` (`src-tauri/src/ai/`) is the single seam, shaped like `provider.rs`: an object-safe, synchronous trait, an `ai::harness_for()` resolver, a compile-time object-safety assertion and a recipe comment for adding an implementation. `ai/claude.rs` (`ClaudeCode`) is the only code that builds a `claude` command line or parses Claude's output (the `-p` JSON envelope and the `stream-json` lines).

Requests are neutral: tool policy, access level, session mode (ephemeral, new id, resume, fork), system prompt mode. Each harness translates them to its own flags. Capabilities are data (`Caps`), so the UI and routers can refuse unsupported combinations instead of failing mid-run.

Decisions that are not obvious from the code:

- **No behavior change when introduced.** Argv and env of every call site are pinned by snapshot tests that were written before the move. `Access::Native` passes a harness-native permission mode through verbatim because routines still store Claude's `permission_mode` strings; it goes away when profiles replace those columns.
- **`spawn` returns the `Child`, not a bare pid.** The run engine writes the prompt to stdin and polls and reaps the process, so a pid is not enough. Streaming still goes to files so a run can outlive the app.
- **Auth is delegated to each CLI.** The app stores no token and has no raw HTTP client. Model providers reachable through a CLI (for example OpenRouter via opencode) are reached through that CLI.
- **Profiles select the harness.** A profile names a harness plus model and settings (`ai_profile`, never a secret). A routine uses its own profile, else the profile mapped to its feature (`meta` rows `ai_profile.chat|summaries|theme|sessions`), else the default profile (`is_default`, at most one). A launch can override the model only. A missing or deleted profile falls back to the default; with no default the app runs Claude Code with the legacy env-based model resolution, so Bedrock setups keep working. `ai::profiles` holds storage and the resolver; `harness_by_id()` is the registry. The run engine itself still takes one harness at construction until a second harness exists.
- **Cheap built-in models stay cheap.** Summaries and theme suggestions ship their own fast model. The default profile's model never applies to them: only a profile mapped to that feature with a model of its own does.
- **Routines get a neutral access model.** `routine.access` (`read_only`, `edit`, `full`) is the primary setting; Claude's permission mode, allowed tools, extra args and system prompt mode move to `routine.harness_overrides` keyed by harness id. Migration 0043 backfills both from the old columns, which stay for one release (still written, no longer read) and are dropped later. A native permission mode wins over the access, so changing only the access drops a stale native mode. Before a launch, `ai::check_caps` refuses a routine whose access or tool needs exceed the harness `Caps`, naming the missing capability.
- **Routine kind.** `plain` or `system` (`system` = has a Rust handler). System routines run locally and expose only profile, model, timeout and enabled; their prompts and tools belong to the handler.
- **Chats pin their profile.** `chat.profile_id` is set when a session starts. Switching profile starts a new harness session and puts the earlier messages in the first turn's system prompt as a previous-conversation block capped at about 200 KB (oldest messages dropped first, with a marker line). Sessions are only resumed on the profile they started with.
- **Session discovery stays Claude-specific** (`claude_sessions.rs`). `Harness::live_sessions` exists with an empty default so multi-harness session listing can be added without another trait change.

## Considered Options

- A raw HTTP client for API-key models. Rejected: it would make the app store tokens and re-implement tool use, session handling and cost reporting that the CLIs already provide.
- An enum of harnesses instead of a trait. Rejected: `provider.rs` already uses a trait for the forge, and a trait lets a remote wrapper implement the same interface later.
