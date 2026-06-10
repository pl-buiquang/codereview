// Reviews screen — sidebar + filter rail + review rows
const ReviewsScreen = () => {
  const rows = [
    {
      title: "feat(result-editor): add warning banner inside TipTap editor",
      repo: "git@github-philips:philips-internal/cardiologs-front",
      pr: "PR #12706", comments: "0 comments", verdict: "comment", when: "updated 3d ago",
    },
    {
      title: "feat(setting): add Automation Manager settings sub-page",
      repo: "git@github-philips:philips-internal/cardiologs-front",
      pr: "PR #12709", comments: "0 comments", verdict: null, when: "updated 5d ago",
    },
  ];
  return (
    <WindowChrome activeTab={-1}>
      <AppSidebar active="reviews" />
      <div className="cr-main">
        <div className="cr-pagehead">
          <div>
            <h1 className="cr-h1">Reviews</h1>
            <div className="cr-sub">2 reviews</div>
          </div>
          <span className="cr-spacer"></span>
          <span className="faint" style={{ fontSize: 11.5, alignSelf: "center" }}>Sort</span>
          <SelectBox value="Last modified" style={{ width: 150 }} />
        </div>
        <div style={{ display: "flex", flex: 1, minHeight: 0, marginTop: 6 }}>
          <div className="cr-rail">
            <div className="cr-rail-h">Status</div>
            <span className="cr-rail-item on"><span className="lbl">Draft</span><span className="count">2</span></span>
            <div className="cr-rail-h">Origin</div>
            <span className="cr-rail-item"><span className="lbl">GitHub</span><span className="count">2</span></span>
            <div className="cr-rail-h">Repositories</div>
            <span className="cr-rail-item"><span className="lbl mono" style={{ fontSize: 11 }}>git@github-philips…</span><span className="count">2</span></span>
            <div className="cr-rail-h">Verdict</div>
            <span className="cr-rail-item"><span className="lbl">Comment</span><span className="count">1</span></span>
            <span className="cr-rail-item"><span className="lbl">No verdict</span><span className="count">1</span></span>
          </div>
          <div className="cr-list" style={{ flex: 1, minWidth: 0, paddingLeft: 8 }}>
            {rows.map((r, i) => (
              <div key={i} className="card rev-row">
                <div className="rev-main">
                  <div className="rev-title">{r.title}</div>
                  <div className="rev-meta">
                    <span className="mono faint" style={{ fontSize: 11 }}>{r.repo}</span>
                    <span className="sep"></span><span>{r.pr}</span>
                    <span className="sep"></span><span>{r.comments}</span>
                    {r.verdict ? <React.Fragment><span className="sep"></span><span style={{ color: "var(--accent)" }}>{r.verdict}</span></React.Fragment> : null}
                    <span className="sep"></span><span className="faint">{r.when}</span>
                  </div>
                </div>
                <Badge kind="draft">draft</Badge>
                <SplitBtn sm>Open PR<CRIcon name="ext" size={10} /></SplitBtn>
                <span className="btn btn-sm btn-ghost"><CRIcon name="x" size={11} /></span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </WindowChrome>
  );
};

window.ReviewsScreen = ReviewsScreen;
