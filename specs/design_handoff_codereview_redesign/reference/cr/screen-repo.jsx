// Repo / Virtual PR screen — branch pickers + reviews list
const RepoScreen = () => {
  return (
    <WindowChrome activeTab={0}>
      <div className="cr-main">
        <div style={{ padding: "20px 28px 0", display: "flex", flexDirection: "column", gap: 0, flex: 1, minHeight: 0 }}>
          <div className="row" style={{ gap: 10 }}>
            <CRIcon name="repo" size={16} style={{ color: "var(--text-2)" }} />
            <h1 className="cr-h1" style={{ fontSize: 15 }}>git@github-philips:philips-internal/cardiologs-front</h1>
          </div>
          <div className="cr-tabs" style={{ padding: 0, marginTop: 16 }}>
            <span className="cr-tab active">Virtual PR</span>
            <span className="cr-tab">GitHub PRs</span>
          </div>

          <div className="card" style={{ marginTop: 18, padding: "16px 18px", display: "flex", flexDirection: "column", gap: 14 }}>
            <div className="row" style={{ gap: 12 }}>
              <span style={{ fontWeight: 700, fontSize: 13 }}>New virtual PR</span>
              <span className="faint" style={{ fontSize: 11.5 }}>base</span>
              <SelectBox mono value="main" style={{ width: 320 }} />
              <span className="faint" style={{ fontSize: 12 }}>← compare</span>
              <SelectBox mono value="feature/pte-filter-associated-episodes" style={{ flex: 1 }} />
              <Check on label="merge-base" />
            </div>
            <div className="row">
              <Btn><CRIcon name="branch" size={13} />Preview diff</Btn>
              <Btn kind="primary">Start review</Btn>
              <span className="cr-spacer"></span>
              <Seg items={["Split", "Unified"]} on="Split" />
            </div>
            <div className="faint" style={{ fontSize: 11.5 }}>Pick branches, then “Preview diff” or “Start review”.</div>
          </div>

          <div style={{ fontWeight: 700, fontSize: 14, margin: "22px 0 10px", fontFamily: "var(--font-display)" }}>Reviews</div>
          <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
            <div className="card rev-row">
              <div className="rev-main">
                <div className="rev-title">feat(result-editor): add warning banner inside TipTap editor</div>
                <div className="rev-meta"><span>PR #12706</span><span className="sep"></span><span>0 comments</span><span className="sep"></span><span style={{ color: "var(--accent)" }}>comment</span></div>
              </div>
              <Badge kind="draft">draft</Badge>
              <SplitBtn sm>Open PR<CRIcon name="ext" size={10} /></SplitBtn>
              <span className="btn btn-sm btn-ghost"><CRIcon name="x" size={11} /></span>
            </div>
            <div className="card rev-row">
              <div className="rev-main">
                <div className="rev-title">feat(setting): add Automation Manager settings sub-page</div>
                <div className="rev-meta"><span>PR #12709</span><span className="sep"></span><span>0 comments</span></div>
              </div>
              <Badge kind="draft">draft</Badge>
              <SplitBtn sm>Open PR<CRIcon name="ext" size={10} /></SplitBtn>
              <span className="btn btn-sm btn-ghost"><CRIcon name="x" size={11} /></span>
            </div>
          </div>
        </div>
      </div>
    </WindowChrome>
  );
};

window.RepoScreen = RepoScreen;
