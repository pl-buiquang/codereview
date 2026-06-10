// Diff editor screen 2 — PR header, inline comment thread, composer
const DiffCommentsScreen = () => {
  const T = (c, t) => <span className={"tok-" + c}>{t}</span>;

  const leftLines = [
    [7, "ctx", "    canActivateAuth,"],
    [8, "ctx", "    canActivateFolder,"],
    [9, "ctx", "    canActivateFoldersLoaded,"],
    [null, "empty", ""],
    [10, "ctx", "    canActivateManager,"],
    [11, "ctx", "    canActivateMembersLoaded,"],
    [12, "ctx", "    canActivateOrganizationCommentsGuard,"],
  ];
  const rightLines = [
    [7, "ctx", "    canActivateAuth,"],
    [8, "ctx", "    canActivateFolder,"],
    [9, "ctx", "    canActivateFoldersLoaded,"],
    [10, "add", "    canActivateHasFeatureFlagOrRedirect,"],
    [11, "ctx", "    canActivateManager,"],
    [12, "ctx", "    canActivateMembersLoaded,"],
    [13, "ctx", "    canActivateOrganizationCommentsGuard,"],
  ];
  const leftLines2 = [
    [41, "ctx", <React.Fragment>{T("tag", "import")}{" { ShareInsightsAccessComponent } "}{T("tag", "from")} {T("str", "'./components/share-insights-access…'")};</React.Fragment>],
    [42, "ctx", <React.Fragment>{T("tag", "import")}{" { SignalInsightsManagerComponent } "}{T("tag", "from")} {T("str", "'./components/signal-insights-manager…'")};</React.Fragment>],
    [null, "empty", ""],
    [43, "ctx", <React.Fragment>{T("tag", "import")}{" { CustomReportTemplatesComponent } "}{T("tag", "from")} {T("str", "'./components/custom-report-templates…'")};</React.Fragment>],
  ];
  const rightLines2 = [
    [42, "ctx", <React.Fragment>{T("tag", "import")}{" { ShareInsightsAccessComponent } "}{T("tag", "from")} {T("str", "'./components/share-insights-access…'")};</React.Fragment>],
    [43, "ctx", <React.Fragment>{T("tag", "import")}{" { SignalInsightsManagerComponent } "}{T("tag", "from")} {T("str", "'./components/signal-insights-manager…'")};</React.Fragment>],
    [44, "add", <React.Fragment>{T("tag", "import")}{" { AutomationManagerComponent } "}{T("tag", "from")} {T("str", "'./components/automation-manager…'")};</React.Fragment>],
    [45, "ctx", <React.Fragment>{T("tag", "import")}{" { CustomReportTemplatesComponent } "}{T("tag", "from")} {T("str", "'./components/custom-report-templates…'")};</React.Fragment>],
  ];

  const tree = [
    { d: 0, dir: true, label: "i18n" },
    { d: 1, label: "cs.json", add: "+6", del: "−0" },
    { d: 1, label: "da.json", add: "+6", del: "−0" },
    { d: 1, label: "de.json", add: "+6", del: "−0" },
    { d: 1, label: "en.json", add: "+6", del: "−0" },
    { d: 1, label: "es.json", add: "+6", del: "−0" },
    { d: 1, label: "fr.json", add: "+6", del: "−0" },
    { d: 0, dir: true, label: "libs" },
    { d: 1, dir: true, label: "account-page/src/lib" },
    { d: 2, label: "account-page.modul…", add: "+2", del: "−0" },
    { d: 2, label: "account.router.ts", add: "+14", del: "−0", on: true, n: 1 },
    { d: 2, dir: true, label: "components" },
    { d: 3, dir: true, label: "automation-manag…" },
    { d: 4, label: "automation-man…", add: "+43", del: "−0" },
    { d: 4, label: "automation-ma…", add: "+164", del: "−0" },
    { d: 4, label: "automation-man…", add: "+91", del: "−0" },
    { d: 3, dir: true, label: "settings" },
    { d: 4, label: "settings.compo…", add: "+32", del: "−0" },
    { d: 4, label: "settings.compon…", add: "+5", del: "−0" },
  ];

  const Col = ({ lines }) => (
    <div className="diff-col">
      {lines.map((l, i) => (
        <div key={i} className={"dline " + l[1]}>
          <span className="dnum">{l[0] || ""}</span>
          <span className="dcode">{l[2]}</span>
        </div>
      ))}
    </div>
  );
  const Code = (t) => <span className="code-chip">{t}</span>;

  return (
    <WindowChrome activeTab={1}>
      <div className="cr-main">
        {/* toolbar */}
        <div className="row" style={{ padding: "12px 16px 0", gap: 8 }}>
          <Btn><CRIcon name="back" size={12} />Back</Btn>
          <Btn kind="ghost">Collapse</Btn>
          <div style={{ fontFamily: "var(--font-mono)", fontWeight: 700, fontSize: 13.5, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            feat(setting): add Automation Manager settings sub-page
          </div>
          <Badge kind="draft">draft</Badge>
          <span className="faint" style={{ fontSize: 11.5 }}>Saved</span>
          <span className="cr-spacer"></span>
          <Seg items={["Split", "Unified"]} on="Split" />
          <SplitBtn>Open PR<CRIcon name="ext" size={11} /></SplitBtn>
          <Btn><CRIcon name="refresh" size={12} />Refresh</Btn>
          <Btn>Export</Btn>
          <Btn kind="primary">Publish</Btn>
          <Btn kind="danger">Delete</Btn>
        </div>
        {/* PR header */}
        <div style={{ padding: "10px 16px 0" }}>
          <div className="card prh">
            <div className="prh-status">
              <Badge kind="open">open</Badge>
              <span className="chip bad">✕ ci</span>
              <span style={{ fontWeight: 600, color: "var(--text)" }}>Checking…</span>
              <span><span className="delta-add">+446</span> <span className="delta-del">−0</span></span>
              <span className="faint">· 25 files</span>
              <span className="cr-spacer"></span>
              <span className="prh-checks">▸ 37 checks</span>
            </div>
            <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
              <span className="prh-approved">Approved</span>
              <span className="avatar-chip"><Avatar size={16} />NTurchi</span>
              <span className="avatar-chip"><Avatar size={16} />ToomeyDamien</span>
              <span className="avatar-chip"><Avatar bot size={16} />copilot-pull-request-reviewer</span>
              <span className="avatar-chip"><Avatar size={16} />alainkp</span>
              <span style={{ width: 6 }}></span>
              <span className="label-tag qa">needs-qa</span>
              <span className="label-tag deploy">deploy-front-pr</span>
            </div>
            <div className="prh-desc-h">Description</div>
            <div className="prh-desc">
              Adds the <b>Automation Manager</b> settings sub-page under the Management section, as specified in issue #12694.
              A manager can select the organization's default diagnostic template from a dropdown of all shared templates. Closes #12694
              <span style={{ color: "var(--accent)", marginLeft: 8 }}>Show more</span>
            </div>
          </div>
        </div>
        {/* body */}
        <div style={{ display: "flex", flex: 1, minHeight: 0, marginTop: 10, borderTop: "1px solid var(--border)" }}>
          <div className="tree">
            <div className="tree-h">Files <span className="faint mono">(25)</span></div>
            {tree.map((t, i) => (
              <span key={i} className={"tree-row" + (t.dir ? " dir" : "") + (t.on ? " on" : "")} style={{ paddingLeft: 8 + t.d * 11 }}>
                <CRIcon name={t.dir ? "folder" : "file"} size={11} />
                <span className="lbl">{t.label}</span>
                {t.add ? <span className="deltas">{t.n ? <span className="badge badge-pr" style={{ height: 14, padding: "0 5px" }}>1</span> : null}<span className="delta-add">{t.add}</span><span className="delta-del">{t.del}</span></span> : null}
              </span>
            ))}
          </div>
          <div className="diff-wrap" style={{ gap: 8 }}>
            <div className="card diff-card" style={{ flex: "none" }}>
              <div className="diff-filehead">
                <span className="path">libs/account-page/src/lib/account.router.ts</span>
                <span><span className="delta-add">+14</span> <span className="delta-del">−0</span></span>
                <Btn sm><CRIcon name="comment" size={11} />Comment on file</Btn>
                <Btn sm>View file</Btn>
                <Btn sm>Open</Btn>
                <Check label="Viewed" />
              </div>
              <div className="thread">
                <div className="comment">
                  <div className="comment-head">
                    <Avatar size={18} /><span className="who">NTurchi</span><span className="when">4d ago</span>
                    <a className="gh" href="#">View on GitHub</a>
                  </div>
                  <div className="comment-body">
                    Both do 2 different things — {Code("canActivatePreferences")} seems to check if your preferences are up to date;
                    {" "}{Code("canActivateHasFeatureFlagOrRedirect")} prevents access by changing the {Code("url")} if the feature flag is {Code("false")}.
                  </div>
                </div>
                <div className="comment">
                  <div className="comment-head">
                    <Avatar size={18} /><span className="who">pl-buiquang</span><span className="when">1d ago</span>
                    <a className="gh" href="#">View on GitHub</a>
                  </div>
                  <div className="comment-body">
                    {Code("canActivatePreferences")} is a more generic guard. I think we should refactor so that
                    {" "}{Code("canActivateHasFeatureFlagOrRedirect")} reuses its logic — and fix the non-existing route {Code("/account/profile")}.
                    {" "}Tracked in <a href="#">#12725</a>.
                  </div>
                </div>
              </div>
              <div className="diff-cols" style={{ flex: "none" }}>
                <Col lines={leftLines} />
                <Col lines={rightLines} />
              </div>
              <div className="composer">
                <div className="composer-tabs">
                  <span className="composer-tab on">Write</span>
                  <span className="composer-tab">Preview</span>
                </div>
                <div className="composer-body">test</div>
                <div className="composer-foot">
                  <Btn sm>Cancel</Btn>
                  <Btn sm kind="primary">Add comment</Btn>
                </div>
              </div>
              <div className="diff-cols" style={{ flex: "none", borderTop: "1px solid var(--border)" }}>
                <Col lines={leftLines2} />
                <Col lines={rightLines2} />
              </div>
            </div>
          </div>
        </div>
      </div>
    </WindowChrome>
  );
};

window.DiffCommentsScreen = DiffCommentsScreen;
