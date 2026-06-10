// CodeReview redesign — shared chrome & atoms
// Exposes: CRIcon, WindowChrome, AppSidebar, Btn, Seg, Badge, Avatar, Check, Radio, SelectBox

const CRIcon = ({ name, size = 14, style }) => {
  const s = { width: size, height: size, flex: "none", ...style };
  const P = (d) => (
    <svg style={s} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round">{d}</svg>
  );
  switch (name) {
    case "menu": return P(<g><path d="M2.5 4.5h11M2.5 8h11M2.5 11.5h11"></path></g>);
    case "home": return P(<g><path d="M2.5 7.5 8 2.8l5.5 4.7"></path><path d="M3.8 7v6h8.4V7"></path></g>);
    case "x": return P(<g><path d="m4 4 8 8M12 4l-8 8"></path></g>);
    case "plus": return P(<g><path d="M8 3v10M3 8h10"></path></g>);
    case "min": return P(<g><path d="M3 11.5h10"></path></g>);
    case "max": return P(<g><rect x="3.5" y="3.5" width="9" height="9" rx="1"></rect></g>);
    case "inbox": return P(<g><path d="M2.5 9.5h3.2l1 1.8h2.6l1-1.8h3.2"></path><path d="M3.6 3.5h8.8l1.1 6v3h-11v-3z"></path></g>);
    case "review": return P(<g><path d="M3.5 2.5h9v11h-9z"></path><path d="m5.8 8.2 1.5 1.5 3-3.4"></path></g>);
    case "archive": return P(<g><rect x="2.5" y="3" width="11" height="3" rx="0.8"></rect><path d="M3.5 6v7h9V6M6.5 8.5h3"></path></g>);
    case "repo": return P(<g><path d="M4.5 2.5h8v11h-8a1.5 1.5 0 0 1-1.5-1.5V4a1.5 1.5 0 0 1 1.5-1.5Z"></path><path d="M3 10.5h9.5"></path></g>);
    case "gear": return P(<g><circle cx="8" cy="8" r="2.2"></circle><path d="M8 1.8v2M8 12.2v2M1.8 8h2M12.2 8h2M3.6 3.6l1.4 1.4M11 11l1.4 1.4M12.4 3.6 11 5M5 11l-1.4 1.4"></path></g>);
    case "refresh": return P(<g><path d="M13 8a5 5 0 1 1-1.6-3.7"></path><path d="M13 2.8v2.8h-2.8"></path></g>);
    case "chev": return P(<g><path d="m4.5 6.5 3.5 3.5L11.5 6.5"></path></g>);
    case "check": return P(<g><path d="m3.5 8.5 3 3 6-6.5"></path></g>);
    case "back": return P(<g><path d="M9.5 3.5 5 8l4.5 4.5"></path></g>);
    case "ext": return P(<g><path d="M6.5 3.5h-3v9h9v-3"></path><path d="M9 2.5h4.5V7M13 3 7.5 8.5"></path></g>);
    case "comment": return P(<g><path d="M2.5 3.5h11v7h-6L4 13v-2.5H2.5z"></path></g>);
    case "eye": return P(<g><path d="M1.8 8S4 4.2 8 4.2 14.2 8 14.2 8 12 11.8 8 11.8 1.8 8 1.8 8Z"></path><circle cx="8" cy="8" r="1.8"></circle></g>);
    case "branch": return P(<g><circle cx="4.5" cy="3.8" r="1.5"></circle><circle cx="4.5" cy="12.2" r="1.5"></circle><circle cx="11.5" cy="6" r="1.5"></circle><path d="M4.5 5.3v5.4M11.5 7.5c0 2.5-4 2-6.4 2.6"></path></g>);
    case "file": return P(<g><path d="M4 1.8h5.5L12 4.3v9.9H4z"></path><path d="M9.2 2v2.6H12"></path></g>);
    case "folder": return P(<g><path d="M1.8 3.5h4.4l1.2 1.6h6.8v7.4H1.8z"></path></g>);
    case "bot": return P(<g><rect x="3" y="5" width="10" height="7.5" rx="1.5"></rect><path d="M8 2.5V5M6 8.2h.01M10 8.2h.01"></path></g>);
    case "person": return P(<g><circle cx="8" cy="5.2" r="2.6"></circle><path d="M2.8 13.6a5.3 5.3 0 0 1 10.4 0"></path></g>);
    case "flame": return P(<g><path d="M8 1.8C9 4 12 5.5 12 9a4 4 0 0 1-8 0c0-1.5.6-2.5 1.4-3.6C5.8 6.6 7 7 7 7c-.4-2 .2-3.8 1-5.2Z"></path></g>);
    case "team": return P(<g><circle cx="5.5" cy="5.5" r="2"></circle><circle cx="10.8" cy="6.2" r="1.6"></circle><path d="M1.8 12.8a4 4 0 0 1 7.4 0M9.8 12.8a3.4 3.4 0 0 1 4.4-2.4"></path></g>);
    case "closed": return P(<g><circle cx="8" cy="8" r="5.7"></circle><path d="m5.6 8.3 1.7 1.7 3.2-3.6"></path></g>);
    case "sort": return P(<g><path d="M4.5 3v10M4.5 13 2.5 11M4.5 13l2-2M11.5 13V3M11.5 3l-2 2M11.5 3l2 2"></path></g>);
    case "dot": return (<svg style={s} viewBox="0 0 16 16" fill="currentColor"><circle cx="8" cy="8" r="3"></circle></svg>);
    default: return null;
  }
};

// ----- atoms -----
const Btn = ({ kind, sm, children, style }) => (
  <span className={"btn" + (kind ? " btn-" + kind : "") + (sm ? " btn-sm" : "")} style={style}>{children}</span>
);
const SplitBtn = ({ children, sm }) => (
  <span className="btn-split">
    <span className={"btn" + (sm ? " btn-sm" : "")}>{children}</span>
    <span className={"btn" + (sm ? " btn-sm" : "")}><CRIcon name="chev" size={11} /></span>
  </span>
);
const Seg = ({ items, on }) => (
  <span className="seg">
    {items.map((it) => <span key={it} className={"seg-item" + (it === on ? " on" : "")}>{it}</span>)}
  </span>
);
const Badge = ({ kind, children }) => <span className={"badge badge-" + kind}>{children}</span>;
const Avatar = ({ bot, size = 30 }) => (
  <span className="avatar" style={{ width: size, height: size }}>
    <CRIcon name={bot ? "bot" : "person"} size={Math.round(size * 0.55)} />
  </span>
);
const Check = ({ on, label }) => (
  <span className="check">
    <span className={"checkbox" + (on ? " on" : "")}>{on ? <CRIcon name="check" size={9} /> : null}</span>
    {label}
  </span>
);
const Radio = ({ on, label }) => (
  <span className="check"><span className={"radio" + (on ? " on" : "")}></span>{label}</span>
);
const SelectBox = ({ value, mono, style }) => (
  <span className="select" style={style}>
    <span className={mono ? "mono" : ""} style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{value}</span>
    <CRIcon name="chev" size={12} style={{ color: "var(--text-3)" }} />
  </span>
);

// ----- window chrome -----
const WindowChrome = ({ activeTab = 0, children }) => {
  const tabs = [
    "git@github-philips:philips-i…",
    "feat(setting): add Automa…",
    "feat(result-editor): add wa…",
  ];
  return (
    <div className="crw" style={{ display: "flex", flexDirection: "column", width: "100%", height: "100%" }}>
      <div className="crw-titlebar">
        <span className="crw-menu"><CRIcon name="menu" size={15} /></span>
        <span className="crw-home" style={{ alignSelf: "center" }}><CRIcon name="home" size={15} /></span>
        <div className="crw-tabs">
          {tabs.map((t, i) => (
            <span key={t} className={"crw-tab" + (i === activeTab ? " active" : "")}>
              {i === activeTab ? <span className="crw-tab-dot"></span> : <CRIcon name={i === 0 ? "repo" : "review"} size={12} style={{ color: "var(--text-3)" }} />}
              <span className="crw-tab-label mono" style={{ fontSize: 11.5 }}>{t}</span>
              <span className="crw-x"><CRIcon name="x" size={10} /></span>
            </span>
          ))}
          <span className="crw-new"><CRIcon name="plus" size={13} /></span>
        </div>
        <div className="crw-winctl">
          <CRIcon name="min" size={13} />
          <CRIcon name="max" size={12} />
          <CRIcon name="x" size={13} />
        </div>
      </div>
      <div className="crw-body">{children}</div>
    </div>
  );
};

// ----- app sidebar -----
const AppSidebar = ({ active = "inbox" }) => {
  const items = [
    { id: "inbox", icon: "inbox", label: "Inbox", count: 37 },
    { id: "reviews", icon: "review", label: "Reviews", count: 2 },
    { id: "archive", icon: "archive", label: "Archive" },
    { id: "repos", icon: "repo", label: "Repositories" },
  ];
  return (
    <div className="cr-side">
      <div className="cr-side-brand">
        <span className="cr-side-logo">cr</span>
        codereview
      </div>
      <div className="cr-nav">
        {items.map((it) => (
          <span key={it.id} className={"cr-nav-item" + (it.id === active ? " active" : "")}>
            <CRIcon name={it.icon} size={15} />
            {it.label}
            {it.count != null ? <span className="count">{it.count}</span> : null}
          </span>
        ))}
      </div>
      <div className="cr-side-foot cr-nav">
        <span className="cr-nav-item"><CRIcon name="gear" size={15} />Settings</span>
      </div>
    </div>
  );
};

Object.assign(window, {
  CRIcon, Btn, SplitBtn, Seg, Badge, Avatar, Check, Radio, SelectBox, WindowChrome, AppSidebar,
});
