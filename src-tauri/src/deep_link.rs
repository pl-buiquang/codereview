use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DeepLinkAction {
    OpenReview {
        review_id: i64,
    },
    OpenPr {
        owner: String,
        name: String,
        number: i64,
    },
    OpenDiff {
        repo_id: i64,
        base_ref: String,
        head_ref: String,
        three_dot: bool,
    },
    NavigateHome {
        section: Option<String>,
    },
}

/// Parse a `codereview://` URL into a navigation action.
pub fn parse_deep_link(raw: &str) -> Result<DeepLinkAction, String> {
    let url = url::Url::parse(raw).map_err(|e| format!("invalid URL: {e}"))?;

    if url.scheme() != "codereview" {
        return Err(format!("unexpected scheme: {}", url.scheme()));
    }

    // The `url` crate treats custom-scheme URLs as opaque: the first path
    // component ends up in `host_str()`. Reconstruct a full segment list.
    let host = url.host_str().unwrap_or("");
    let path_tail = url.path().trim_start_matches('/');
    let segments: Vec<&str> = std::iter::once(host)
        .chain(path_tail.split('/').filter(|s| !s.is_empty()))
        .collect();

    match segments.as_slice() {
        ["review", id] => {
            let review_id = id
                .parse::<i64>()
                .map_err(|_| format!("invalid review id: {id}"))?;
            Ok(DeepLinkAction::OpenReview { review_id })
        }
        ["pr", owner, name, number] => {
            let number = number
                .parse::<i64>()
                .map_err(|_| format!("invalid PR number: {number}"))?;
            Ok(DeepLinkAction::OpenPr {
                owner: (*owner).to_string(),
                name: (*name).to_string(),
                number,
            })
        }
        // diff uses query params because paths can contain slashes
        ["diff"] => parse_diff_params(&url),
        ["home"] => Ok(DeepLinkAction::NavigateHome { section: None }),
        ["home", section] => Ok(DeepLinkAction::NavigateHome {
            section: Some((*section).to_string()),
        }),
        _ => Err(format!("unknown path: {}", segments.join("/"))),
    }
}

fn parse_diff_params(url: &url::Url) -> Result<DeepLinkAction, String> {
    let pairs: std::collections::HashMap<std::borrow::Cow<str>, std::borrow::Cow<str>> =
        url.query_pairs().collect();
    let repo_id = pairs
        .get("repo")
        .ok_or("missing 'repo' query param")?
        .parse::<i64>()
        .map_err(|_| "invalid 'repo' id")?;
    let base_ref = pairs
        .get("base")
        .ok_or("missing 'base' query param")?
        .to_string();
    let head_ref = pairs
        .get("head")
        .ok_or("missing 'head' query param")?
        .to_string();
    crate::git::validate_commit_ref(&base_ref).map_err(|e| e.to_string())?;
    crate::git::validate_commit_ref(&head_ref).map_err(|e| e.to_string())?;
    let three_dot = pairs
        .get("three_dot")
        .is_none_or(|v| v != "false" && v != "0");
    Ok(DeepLinkAction::OpenDiff {
        repo_id,
        base_ref,
        head_ref,
        three_dot,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_diff_refs() {
        for field in ["base", "head"] {
            for value in ["--output=/", "--all", "", "main\nfeature", "main\0"] {
                let mut url = url::Url::parse("codereview://diff").unwrap();
                url.query_pairs_mut()
                    .append_pair("repo", "1")
                    .append_pair("base", if field == "base" { value } else { "main" })
                    .append_pair("head", if field == "head" { value } else { "feature" })
                    .append_pair("three_dot", "false");
                assert!(parse_deep_link(url.as_str()).is_err());
            }
        }
    }

    #[test]
    fn parse_review() {
        let a = parse_deep_link("codereview://review/42").unwrap();
        let DeepLinkAction::OpenReview { review_id } = a else {
            panic!("expected OpenReview, got {a:?}");
        };
        assert_eq!(review_id, 42);
    }

    #[test]
    fn parse_pr() {
        let a = parse_deep_link("codereview://pr/acme/widget/99").unwrap();
        let DeepLinkAction::OpenPr {
            owner,
            name,
            number,
        } = a
        else {
            panic!("expected OpenPr, got {a:?}");
        };
        assert_eq!(owner, "acme");
        assert_eq!(name, "widget");
        assert_eq!(number, 99);
    }

    #[test]
    fn parse_diff() {
        let a = parse_deep_link("codereview://diff?repo=5&base=main&head=feature&three_dot=true")
            .unwrap();
        let DeepLinkAction::OpenDiff {
            repo_id,
            base_ref,
            head_ref,
            three_dot,
        } = a
        else {
            panic!("expected OpenDiff, got {a:?}");
        };
        assert_eq!(repo_id, 5);
        assert_eq!(base_ref, "main");
        assert_eq!(head_ref, "feature");
        assert!(three_dot);
    }

    #[test]
    fn parse_diff_defaults_three_dot_true() {
        let a = parse_deep_link("codereview://diff?repo=1&base=a&head=b").unwrap();
        let DeepLinkAction::OpenDiff { three_dot, .. } = a else {
            panic!("expected OpenDiff");
        };
        assert!(three_dot);
    }

    #[test]
    fn parse_home_no_section() {
        let a = parse_deep_link("codereview://home").unwrap();
        let DeepLinkAction::NavigateHome { section } = a else {
            panic!("expected NavigateHome, got {a:?}");
        };
        assert!(section.is_none());
    }

    #[test]
    fn parse_home_with_section() {
        let a = parse_deep_link("codereview://home/inbox").unwrap();
        let DeepLinkAction::NavigateHome { section } = a else {
            panic!("expected NavigateHome, got {a:?}");
        };
        assert_eq!(section.as_deref(), Some("inbox"));
    }

    #[test]
    fn wrong_scheme_errors() {
        assert!(parse_deep_link("https://example.com").is_err());
    }

    #[test]
    fn unknown_path_errors() {
        assert!(parse_deep_link("codereview://unknown/path").is_err());
    }

    #[test]
    fn invalid_review_id_errors() {
        assert!(parse_deep_link("codereview://review/abc").is_err());
    }
}
