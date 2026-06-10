// Diff / review editor screen — toolbar + summary + file tree + split diff
const DiffScreen = () => {
  const T = (c, t) => <span className={"tok-" + c}>{t}</span>;

  // [lineNo|null, "ctx"|"add"|"del"|"empty", content]
  const leftLines = [
    [251, "ctx", <React.Fragment>{"  <"}{T("tag", "div")} {T("attr", "class")}={T("str", '"report-view__information__comments"')}{">"}</React.Fragment>],
    [252, "ctx", <React.Fragment>{"    <"}{T("tag", "span")} {T("attr", "class")}={T("str", '"report-view__title"')}{">"}{T("txt", "{{ 'AMBULATORY.REPORT.RESULTS'")}</React.Fragment>],
    [null, "ctx", <React.Fragment>{T("txt", "      | translate }}")}{"</"}{T("tag", "span")}{">"}</React.Fragment>],
    [253, "del", <React.Fragment>{"    <"}{T("tag", "hb-alert")}</React.Fragment>],
    [254, "del", <React.Fragment>{"      "}{T("attr", "*ngIf")}={T("str", '"inconsistentResultFindingsAlertContent.length > 0')}</React.Fragment>],
    [null, "del", <React.Fragment>{T("str", '        && reportForm.value.resultTemplate.trim().length > 0"')}</React.Fragment>],
    [255, "del", <React.Fragment>{"      "}{T("attr", "type")}={T("str", '"warning"')}</React.Fragment>],
    [256, "del", <React.Fragment>{"      "}{T("attr", "size")}={T("str", '"small"')}</React.Fragment>],
    [257, "del", <React.Fragment>{"      "}{T("attr", "[header]")}={T("str", "\"'AMBULATORY.REPORT.INCONSISTENT…' | translate\"")}</React.Fragment>],
    [258, "del", <React.Fragment>{"    >"}</React.Fragment>],
    [259, "del", <React.Fragment>{"      <"}{T("tag", "div")}{">"}{T("txt", "{{ inconsistentResultFindingsAlertMessage }}")}{"</"}{T("tag", "div")}{">"}</React.Fragment>],
    [260, "del", <React.Fragment>{"      <"}{T("tag", "ul")}{">"}</React.Fragment>],
    [261, "del", <React.Fragment>{"        <"}{T("tag", "li")}{">"}{T("txt", "{{ inconsistentResultFindingsAlertContent }}")}{"</"}{T("tag", "li")}{">"}</React.Fragment>],
    [262, "del", <React.Fragment>{"      </"}{T("tag", "ul")}{">"}</React.Fragment>],
    [263, "del", <React.Fragment>{"    </"}{T("tag", "hb-alert")}{">"}</React.Fragment>],
    [264, "ctx", <React.Fragment>{"    <"}{T("tag", "div")} {T("attr", "class")}={T("str", '"report-view__information__comments__container"')}{">"}</React.Fragment>],
    [265, "ctx", <React.Fragment>{"      <"}{T("tag", "div")}</React.Fragment>],
    [266, "ctx", <React.Fragment>{"        "}{T("attr", "data-testid")}={T("str", '"report-view-information-comments-container"')}</React.Fragment>],
    [270, "ctx", <React.Fragment>{"        <"}{T("tag", "cdl-report-result-editor")}</React.Fragment>],
    [271, "ctx", <React.Fragment>{"          "}{T("attr", "[mentionFeeds]")}={T("str", '"mentionFeeds$ | async"')}</React.Fragment>],
    [272, "ctx", <React.Fragment>{"          "}{T("attr", "[template]")}={T("str", '"reportForm.value.resultTemplate"')}</React.Fragment>],
  ];
  const rightLines = [
    [251, "ctx", <React.Fragment>{"  <"}{T("tag", "div")} {T("attr", "class")}={T("str", '"report-view__information__comments"')}{">"}</React.Fragment>],
    [252, "ctx", <React.Fragment>{"    <"}{T("tag", "span")} {T("attr", "class")}={T("str", '"report-view__title"')}{">"}{T("txt", "{{ 'AMBULATORY.REPORT.RESULTS'")}</React.Fragment>],
    [null, "ctx", <React.Fragment>{T("txt", "      | translate }}")}{"</"}{T("tag", "span")}{">"}</React.Fragment>],
    [null, "empty", ""], [null, "empty", ""], [null, "empty", ""], [null, "empty", ""],
    [null, "empty", ""], [null, "empty", ""], [null, "empty", ""], [null, "empty", ""],
    [null, "empty", ""], [null, "empty", ""], [null, "empty", ""], [null, "empty", ""],
    [253, "ctx", <React.Fragment>{"    <"}{T("tag", "div")} {T("attr", "class")}={T("str", '"report-view__information__comments__container"')}{">"}</React.Fragment>],
    [254, "ctx", <React.Fragment>{"      <"}{T("tag", "div")}</React.Fragment>],
    [255, "ctx", <React.Fragment>{"        "}{T("attr", "data-testid")}={T("str", '"report-view-information-comments-container"')}</React.Fragment>],
    [259, "ctx", <React.Fragment>{"        <"}{T("tag", "cdl-report-result-editor")}</React.Fragment>],
    [260, "ctx", <React.Fragment>{"          "}{T("attr", "[mentionFeeds]")}={T("str", '"mentionFeeds$ | async"')}</React.Fragment>],
    [261, "ctx", <React.Fragment>{"          "}{T("attr", "[template]")}={T("str", '"reportForm.value.resultTemplate"')}</React.Fragment>],
    [262, "add", <React.Fragment>{"          "}{T("attr", "[warning]")}={T("str", '"resultWarningBanner$ | async"')}</React.Fragment>],
  ];

  const tree = [
    { d: 0, dir: true, label: "libs" },
    { d: 1, dir: true, label: "base/src/lib/ambulatory…" },
    { d: 2, label: "report-information.c…", add: "+9", del: "−11", on: true },
    { d: 1, dir: true, label: "report-result-editor" },
    { d: 2, label: "report-result-edito…", add: "+3", del: "−1" },
    { d: 1, dir: true, label: "result-editor/src" },
    { d: 2, label: "index.ts", add: "+1", del: "−1" },
    { d: 2, dir: true, label: "lib/result-editor" },
    { d: 3, dir: true, label: "extensions" },
    { d: 4, label: "warning-banner.ex…", add: "+69", del: "−0" },
    { d: 4, label: "result-editor.comp…", add: "+40", del: "−0" },
    { d: 4, label: "result-editor.comp…", add: "+16", del: "−2" },
    { d: 3, dir: true, label: "utils" },
    { d: 4, label: "tiptap.ts", add: "+2", del: "−0" },
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

  return (
    <WindowChrome activeTab={2}>
      <div className="cr-main">
        {/* toolbar */}
        <div className="row" style={{ padding: "12px 16px 0", gap: 8 }}>
          <Btn><CRIcon name="back" size={12} />Back</Btn>
          <Btn kind="ghost">Collapse</Btn>
          <div style={{ fontFamily: "var(--font-mono)", fontWeight: 700, fontSize: 13.5, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            feat(result-editor): add warning banner inside TipTap editor
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
        {/* banner + summary + verdict */}
        <div style={{ padding: "10px 16px 0", display: "flex", flexDirection: "column", gap: 8 }}>
          <div className="banner">
            <CRIcon name="x" size={13} style={{ marginTop: 1 }} />
            Could not load PR details: gh error: gh: Could not resolve to a Repository with the name 'git@github-philips:philips-internal/cardiologs-front'.
          </div>
          <div className="row" style={{ alignItems: "stretch", gap: 10 }}>
            <div className="textarea" style={{ flex: 1, minHeight: 44 }}>Review summary…</div>
            <div className="card" style={{ padding: "8px 14px", display: "flex", flexDirection: "column", gap: 6, justifyContent: "center" }}>
              <span className="faint" style={{ fontSize: 10.5, fontWeight: 700, letterSpacing: "0.08em", textTransform: "uppercase" }}>Verdict</span>
              <div className="row" style={{ gap: 14 }}>
                <Radio on label="Comment" />
                <Radio label="Approve" />
                <Radio label="Request changes" />
              </div>
            </div>
          </div>
        </div>
        {/* body */}
        <div style={{ display: "flex", flex: 1, minHeight: 0, marginTop: 10, borderTop: "1px solid var(--border)" }}>
          <div className="tree">
            <div className="tree-h">Files <span className="faint mono">(7)</span></div>
            {tree.map((t, i) => (
              <span key={i} className={"tree-row" + (t.dir ? " dir" : "") + (t.on ? " on" : "")} style={{ paddingLeft: 8 + t.d * 12 }}>
                <CRIcon name={t.dir ? "folder" : "file"} size={11} />
                <span className="lbl">{t.label}</span>
                {t.add ? <span className="deltas"><span className="delta-add">{t.add}</span><span className="delta-del">{t.del}</span></span> : null}
              </span>
            ))}
          </div>
          <div className="diff-wrap">
            <div className="diff-hint">Click a line to comment · shift-click another line on the same side to select a range.</div>
            <div className="card diff-card">
              <div className="diff-filehead">
                <span className="path">libs/base/src/lib/ambulatory-holter/components/report-information/report-information.component.html</span>
                <span><span className="delta-add">+9</span> <span className="delta-del">−11</span></span>
                <Btn sm><CRIcon name="comment" size={11} />Comment on file</Btn>
                <Btn sm>View file</Btn>
                <Btn sm>Open</Btn>
                <Check label="Viewed" />
              </div>
              <div className="diff-cols">
                <Col lines={leftLines} />
                <Col lines={rightLines} />
              </div>
            </div>
          </div>
        </div>
      </div>
    </WindowChrome>
  );
};

window.DiffScreen = DiffScreen;
