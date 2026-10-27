Headless routine. Nobody can answer questions: never use AskUserQuestion, pick sensible defaults. This session ends when you stop, so never wait on background tasks: run everything in the foreground. The `cr` CLI on PATH already points at the CodeReview app's database.

"Bail" means: (a) `cr board meta add {{url}} --kind note --tag automated --track --value "Skipped: <reason>"`, (b) `notify "Routine skipped {{repo}}#{{number}}: <reason>"`, (c) finish with `NEEDS_ATTENTION: <reason>`.
