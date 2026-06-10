// Inbox screen — sidebar + tabs + filter rail + PR cards
const InboxScreen = () => {
  const prs = [
    {
      bot: true, repo: "philips-internal/cardiologs-hadrian-ui", num: "#52", ci: false,
      title: "build(deps): bump the version-updates group with 29 updates",
      by: "dependabot", check: "Github Actions validator passed",
      files: "1 file", add: "+3740", del: "−5536", state: "Review required", top: "1 top file", when: "3d ago",
    },
    {
      bot: true, repo: "philips-internal/cardiologs-hadrian-ui", num: "#51", ci: false,
      title: "build(deps): bump the security-updates group across 1 directory with 6 updates",
      by: "dependabot", check: "Github Actions validator passed",
      files: "1 file", add: "+2964", del: "−5101", state: "Review required", top: "1 top file", when: "3d ago",
    },
    {
      bot: true, repo: "philips-internal/cardiologs-hadrian-ui", num: "#53", ci: true,
      title: "build(deps): bump the all-updates group with 2 updates",
      by: "dependabot", check: "Github Actions validator passed",
      files: "2 files", add: "+5", del: "−5", state: "Review required", top: "2 top files", when: "3d ago",
    },
    {
      bot: false, repo: "philips-internal/cardiologs-back", num: "#5636", ci: false,
      title: "fix: outdated structure.sql file",
      by: "johncardiologs", check: "Quality Gate passed · 0 issues · coverage 0.0%",
      files: "1 file", add: "+1", del: "−1", state: "Changes requested", top: "1 top file", when: "4d ago",
    },
    {
      bot: true, repo: "philips-internal/cardiologs-react-reports", num: "#368", ci: true,
      title: "chore(deps): bump the all-updates group across 1 directory with 6 updates",
      by: "dependabot", check: "Github Actions validator passed",
      files: "10 files", add: "+27", del: "−27", state: "Review required", top: "5 top files", when: "5d ago",
    },
  ];
  const repos = [
    ["philips-internal/cardiologs-front", 7], ["philips-internal/cardiologs-back", 5],
    ["philips-internal/hadrian-ui", 4], ["philips-internal/react-reports", 3],
    ["philips-internal/cardiolib", 3], ["powsybl/powsybl-core", 2],
    ["tmangum31/github-tools", 1],
  ];
  const users = [
    ["dependabot", 8], ["pl-buiquang", 8], ["ToomeyDamien", 6],
    ["johncardiologs", 4], ["RobinDjebali", 2], ["alainkp", 1],
  ];
  return (
    <WindowChrome activeTab={-1}>
      <AppSidebar active="inbox" />
      <div className="cr-main">
        <div className="cr-pagehead">
          <div>
            <h1 className="cr-h1">Inbox</h1>
            <div className="cr-sub">Logged in as <span className="mono">@pl-buiquang</span></div>
          </div>
          <span className="cr-spacer"></span>
          <span className="faint" style={{ fontSize: 11.5, alignSelf: "center" }}>updated 3d ago</span>
          <Btn kind="primary"><CRIcon name="refresh" size={13} />Refresh</Btn>
        </div>
        <div className="cr-tabs">
          <span className="cr-tab active"><CRIcon name="flame" size={13} />Needs you<span className="n">37</span></span>
          <span className="cr-tab"><CRIcon name="person" size={13} />Authored<span className="n">46</span></span>
          <span className="cr-tab"><CRIcon name="team" size={13} />Team review<span className="n">36</span></span>
          <span className="cr-tab"><CRIcon name="bot" size={13} />Bots<span className="n">0</span></span>
          <span className="cr-tab"><CRIcon name="eye" size={13} />Visited<span className="n">2</span></span>
          <span className="cr-tab"><CRIcon name="closed" size={13} />Closed<span className="n">0</span></span>
        </div>
        <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
          <div className="cr-rail">
            <div className="cr-rail-h">Type</div>
            <span className="cr-rail-item"><span className="lbl">issue</span><span className="count">18</span></span>
            <span className="cr-rail-item on"><span className="lbl">pr</span><span className="count">19</span></span>
            <div className="cr-rail-h">Repositories</div>
            {repos.map(([r, n]) => (
              <span key={r} className="cr-rail-item"><span className="lbl mono" style={{ fontSize: 11 }}>{r}</span><span className="count">{n}</span></span>
            ))}
            <div className="cr-rail-h">Users</div>
            {users.map(([u, n]) => (
              <span key={u} className="cr-rail-item"><span className="lbl">{u}</span><span className="count">{n}</span></span>
            ))}
          </div>
          <div className="cr-list" style={{ flex: 1, minWidth: 0, paddingLeft: 8 }}>
            {prs.map((p, i) => (
              <div key={i} className="card pr-card">
                <Avatar bot={p.bot} />
                <div className="pr-main">
                  <div className="pr-meta">
                    <Badge kind="pr">PR</Badge>
                    <span className="repo">{p.repo} <span className="faint">{p.num}</span></span>
                    <Badge kind="open">open</Badge>
                    <span className={"chip " + (p.ci ? "ok" : "bad")}>{p.ci ? "✓" : "✕"} ci</span>
                  </div>
                  <div className="pr-title">{p.title}</div>
                  <div className="pr-checks">
                    <Badge kind="review">review</Badge>
                    <span>by <b>{p.by}</b></span>
                    <span className="faint">·</span>
                    <span className="chip ok"><CRIcon name="check" size={11} />{p.check}</span>
                  </div>
                  <div className="pr-foot">
                    <span>{p.files}</span>
                    <span><span className="delta-add">{p.add}</span> <span className="delta-del">{p.del}</span></span>
                    <span className={p.state === "Changes requested" ? "" : ""} style={p.state === "Changes requested" ? { color: "var(--danger)" } : {}}>{p.state}</span>
                    <a href="#">{p.top}</a>
                  </div>
                </div>
                <div className="pr-side">
                  <span className="pr-when">{p.when}</span>
                  <div className="pr-actions">
                    <Btn sm kind="primary">Open as review</Btn>
                    <Btn sm><CRIcon name="check" size={11} />Done</Btn>
                    <Btn sm kind="ghost"><CRIcon name="x" size={10} />Untrack</Btn>
                  </div>
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </WindowChrome>
  );
};

window.InboxScreen = InboxScreen;
