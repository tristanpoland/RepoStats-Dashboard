use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Json, Response},
    routing::get,
    Router,
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::AppState;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(serve_ui))
        .route("/api/overview", get(api_overview))
        .route("/api/snapshots", get(api_snapshots))
        .route("/api/repos", get(api_repos))
        .route("/api/repo/:owner/:name", get(api_repo_detail))
        .route("/api/activity", get(api_activity))
        .route("/api/backlog", get(api_backlog))
        .route("/api/commits/weekly", get(api_weekly_commits))
        .route("/api/contributors", get(api_contributors))
        .route("/api/pulls", get(api_pulls))
        .route("/api/groups", get(api_groups))
        .route("/api/momentum", get(api_momentum))
        .route("/api/imported-files", get(api_imported_files))
        .with_state(state)
}

// ── HTML Shell ────────────────────────────────────────────────────────────────

async fn serve_ui() -> Html<&'static str> {
    Html(UI_HTML)
}

// ── Query param helpers ───────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct SnapshotQuery {
    snapshot_ts: Option<String>,
    group: Option<String>,
    repo: Option<String>,
    limit: Option<i64>,
}

// ── /api/overview ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct Overview {
    total_repos: i64,
    total_snapshots: i64,
    latest_snapshot: Option<String>,
    earliest_snapshot: Option<String>,
    total_stars: i64,
    total_forks: i64,
    groups: Vec<GroupCount>,
    imported_files: i64,
}

#[derive(Serialize)]
struct GroupCount {
    group: String,
    repos: i64,
}

async fn api_overview(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db = state.db.lock().unwrap();

    let total_repos: i64 = db.query_row(
        "SELECT COUNT(DISTINCT repo) FROM repo_snapshots", [], |r| r.get(0)
    ).unwrap_or(0);

    let total_snapshots: i64 = db.query_row(
        "SELECT COUNT(DISTINCT snapshot_ts) FROM repo_snapshots", [], |r| r.get(0)
    ).unwrap_or(0);

    let latest_snapshot: Option<String> = db.query_row(
        "SELECT MAX(snapshot_ts) FROM repo_snapshots", [], |r| r.get(0)
    ).unwrap_or(None);

    let earliest_snapshot: Option<String> = db.query_row(
        "SELECT MIN(snapshot_ts) FROM repo_snapshots", [], |r| r.get(0)
    ).unwrap_or(None);

    let total_stars: i64 = db.query_row(
        "SELECT COALESCE(SUM(stars),0) FROM repo_snapshots WHERE snapshot_ts = (SELECT MAX(snapshot_ts) FROM repo_snapshots)",
        [], |r| r.get(0)
    ).unwrap_or(0);

    let total_forks: i64 = db.query_row(
        "SELECT COALESCE(SUM(forks),0) FROM repo_snapshots WHERE snapshot_ts = (SELECT MAX(snapshot_ts) FROM repo_snapshots)",
        [], |r| r.get(0)
    ).unwrap_or(0);

    let imported_files: i64 = db.query_row(
        "SELECT COUNT(*) FROM _imported_files", [], |r| r.get(0)
    ).unwrap_or(0);

    let mut stmt = db.prepare(
        "SELECT repo_group, COUNT(DISTINCT repo) FROM repo_snapshots GROUP BY repo_group ORDER BY repo_group"
    ).unwrap();
    let groups: Vec<GroupCount> = stmt.query_map([], |r| {
        Ok(GroupCount { group: r.get(0)?, repos: r.get(1)? })
    }).unwrap().filter_map(|r| r.ok()).collect();

    Json(Overview {
        total_repos, total_snapshots, latest_snapshot, earliest_snapshot,
        total_stars, total_forks, groups, imported_files,
    })
}

// ── /api/snapshots ────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct SnapshotMeta {
    snapshot_ts: String,
    repo_count: i64,
    total_stars: i64,
    total_open_issues: i64,
}

async fn api_snapshots(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    let mut stmt = db.prepare(
        "SELECT snapshot_ts, COUNT(DISTINCT repo), SUM(stars), SUM(open_issues_api)
         FROM repo_snapshots GROUP BY snapshot_ts ORDER BY snapshot_ts DESC LIMIT 50"
    ).unwrap();
    let rows: Vec<SnapshotMeta> = stmt.query_map([], |r| {
        Ok(SnapshotMeta {
            snapshot_ts: r.get(0)?,
            repo_count: r.get(1)?,
            total_stars: r.get(2).unwrap_or(0),
            total_open_issues: r.get(3).unwrap_or(0),
        })
    }).unwrap().filter_map(|r| r.ok()).collect();
    Json(rows)
}

// ── /api/repos ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct RepoSummary {
    repo: String,
    repo_group: String,
    stars: i64,
    forks: i64,
    watchers: i64,
    open_issues_api: i64,
    default_branch: Option<String>,
    pushed_at: Option<String>,
    snapshot_ts: String,
}

async fn api_repos(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SnapshotQuery>,
) -> impl IntoResponse {
    let db = state.db.lock().unwrap();

    let ts = q.snapshot_ts.unwrap_or_else(|| {
        db.query_row("SELECT MAX(snapshot_ts) FROM repo_snapshots", [], |r| r.get(0))
            .unwrap_or_else(|_| "".to_string())
    });

    let mut stmt = db.prepare(
        "SELECT repo, repo_group, stars, forks, watchers, open_issues_api,
                default_branch, pushed_at, snapshot_ts
         FROM repo_snapshots WHERE snapshot_ts = ?1
         ORDER BY stars DESC"
    ).unwrap();

    let rows: Vec<RepoSummary> = stmt.query_map(params![ts], |r| {
        Ok(RepoSummary {
            repo: r.get(0)?,
            repo_group: r.get(1)?,
            stars: r.get(2).unwrap_or(0),
            forks: r.get(3).unwrap_or(0),
            watchers: r.get(4).unwrap_or(0),
            open_issues_api: r.get(5).unwrap_or(0),
            default_branch: r.get(6)?,
            pushed_at: r.get(7)?,
            snapshot_ts: r.get(8)?,
        })
    }).unwrap().filter_map(|r| r.ok()).collect();
    Json(rows)
}

// ── /api/repo/:owner/:name ────────────────────────────────────────────────────

#[derive(Serialize)]
struct RepoDetail {
    snapshots: Vec<RepoSummary>,
    activity: Vec<ActivityRow>,
    backlog: Vec<BacklogRow>,
    commits: Vec<WeeklyCommitRow>,
    contributors: Vec<ContribRow>,
    pulls: Vec<PullRow>,
}

async fn api_repo_detail(
    State(state): State<Arc<AppState>>,
    Path((owner, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    let repo = format!("{}/{}", owner, name);

    let mut stmt = db.prepare(
        "SELECT repo, repo_group, stars, forks, watchers, open_issues_api,
                default_branch, pushed_at, snapshot_ts
         FROM repo_snapshots WHERE repo = ?1 ORDER BY snapshot_ts DESC LIMIT 30"
    ).unwrap();
    let snapshots: Vec<RepoSummary> = stmt.query_map(params![repo], |r| {
        Ok(RepoSummary {
            repo: r.get(0)?, repo_group: r.get(1)?,
            stars: r.get(2).unwrap_or(0), forks: r.get(3).unwrap_or(0),
            watchers: r.get(4).unwrap_or(0), open_issues_api: r.get(5).unwrap_or(0),
            default_branch: r.get(6)?, pushed_at: r.get(7)?, snapshot_ts: r.get(8)?,
        })
    }).unwrap().filter_map(|r| r.ok()).collect();

    let mut stmt = db.prepare(
        "SELECT snapshot_ts, repo, window_days, issues_opened, issues_closed,
                prs_opened, prs_closed, net_issues_delta, items_updated
         FROM activity_windows WHERE repo = ?1 ORDER BY snapshot_ts DESC LIMIT 30"
    ).unwrap();
    let activity: Vec<ActivityRow> = stmt.query_map(params![repo], |r| {
        Ok(ActivityRow {
            snapshot_ts: r.get(0)?, repo: r.get(1)?,
            window_days: r.get(2).unwrap_or(28),
            issues_opened: r.get(3).unwrap_or(0), issues_closed: r.get(4).unwrap_or(0),
            prs_opened: r.get(5).unwrap_or(0), prs_closed: r.get(6).unwrap_or(0),
            net_issues_delta: r.get(7).unwrap_or(0), items_updated: r.get(8).unwrap_or(0),
        })
    }).unwrap().filter_map(|r| r.ok()).collect();

    let ts = snapshots.first().map(|s| s.snapshot_ts.clone()).unwrap_or_default();

    let mut stmt = db.prepare(
        "SELECT snapshot_ts, repo, open_issues, age_lt7d, age_7_30d, age_30_90d,
                age_gt90d, median_age_days
         FROM backlog_health WHERE repo = ?1 AND snapshot_ts = ?2"
    ).unwrap();
    let backlog: Vec<BacklogRow> = stmt.query_map(params![repo, ts], |r| {
        Ok(BacklogRow {
            snapshot_ts: r.get(0)?, repo: r.get(1)?,
            open_issues: r.get(2).unwrap_or(0),
            age_lt7d: r.get(3).unwrap_or(0), age_7_30d: r.get(4).unwrap_or(0),
            age_30_90d: r.get(5).unwrap_or(0), age_gt90d: r.get(6).unwrap_or(0),
            median_age_days: r.get(7).unwrap_or(0.0),
        })
    }).unwrap().filter_map(|r| r.ok()).collect();

    let mut stmt = db.prepare(
        "SELECT snapshot_ts, repo, week_offset, week_label, commits
         FROM weekly_commits WHERE repo = ?1 AND snapshot_ts = ?2
         ORDER BY week_offset DESC"
    ).unwrap();
    let commits: Vec<WeeklyCommitRow> = stmt.query_map(params![repo, ts], |r| {
        Ok(WeeklyCommitRow {
            snapshot_ts: r.get(0)?, repo: r.get(1)?,
            week_offset: r.get(2)?, week_label: r.get(3)?,
            commits: r.get(4).unwrap_or(0),
        })
    }).unwrap().filter_map(|r| r.ok()).collect();

    let mut stmt = db.prepare(
        "SELECT snapshot_ts, repo, login, contributions, share_pct
         FROM contributors WHERE repo = ?1 AND snapshot_ts = ?2
         ORDER BY contributions DESC"
    ).unwrap();
    let contributors: Vec<ContribRow> = stmt.query_map(params![repo, ts], |r| {
        Ok(ContribRow {
            snapshot_ts: r.get(0)?, repo: r.get(1)?,
            login: r.get(2)?, contributions: r.get(3).unwrap_or(0),
            share_pct: r.get(4).unwrap_or(0.0),
        })
    }).unwrap().filter_map(|r| r.ok()).collect();

    let mut stmt = db.prepare(
        "SELECT snapshot_ts, repo, number, title, author, state, updated_at, html_url
         FROM pull_requests WHERE repo = ?1 AND snapshot_ts = ?2
         ORDER BY updated_at DESC"
    ).unwrap();
    let pulls: Vec<PullRow> = stmt.query_map(params![repo, ts], |r| {
        Ok(PullRow {
            snapshot_ts: r.get(0)?, repo: r.get(1)?,
            number: r.get(2)?, title: r.get(3)?,
            author: r.get(4)?, state: r.get(5)?,
            updated_at: r.get(6)?, html_url: r.get(7)?,
        })
    }).unwrap().filter_map(|r| r.ok()).collect();

    Json(RepoDetail { snapshots, activity, backlog, commits, contributors, pulls })
}

// ── /api/activity ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ActivityRow {
    snapshot_ts: String,
    repo: String,
    window_days: i64,
    issues_opened: i64,
    issues_closed: i64,
    prs_opened: i64,
    prs_closed: i64,
    net_issues_delta: i64,
    items_updated: i64,
}

async fn api_activity(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SnapshotQuery>,
) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    let ts = q.snapshot_ts.unwrap_or_else(|| {
        db.query_row("SELECT MAX(snapshot_ts) FROM activity_windows", [], |r| r.get(0))
            .unwrap_or_default()
    });
    let mut stmt = db.prepare(
        "SELECT snapshot_ts, repo, window_days, issues_opened, issues_closed,
                prs_opened, prs_closed, net_issues_delta, items_updated
         FROM activity_windows WHERE snapshot_ts = ?1
         ORDER BY ABS(net_issues_delta) DESC"
    ).unwrap();
    let rows: Vec<ActivityRow> = stmt.query_map(params![ts], |r| {
        Ok(ActivityRow {
            snapshot_ts: r.get(0)?, repo: r.get(1)?,
            window_days: r.get(2).unwrap_or(28),
            issues_opened: r.get(3).unwrap_or(0), issues_closed: r.get(4).unwrap_or(0),
            prs_opened: r.get(5).unwrap_or(0), prs_closed: r.get(6).unwrap_or(0),
            net_issues_delta: r.get(7).unwrap_or(0), items_updated: r.get(8).unwrap_or(0),
        })
    }).unwrap().filter_map(|r| r.ok()).collect();
    Json(rows)
}

// ── /api/backlog ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct BacklogRow {
    snapshot_ts: String,
    repo: String,
    open_issues: i64,
    age_lt7d: i64,
    age_7_30d: i64,
    age_30_90d: i64,
    age_gt90d: i64,
    median_age_days: f64,
}

async fn api_backlog(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SnapshotQuery>,
) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    let ts = q.snapshot_ts.unwrap_or_else(|| {
        db.query_row("SELECT MAX(snapshot_ts) FROM backlog_health", [], |r| r.get(0))
            .unwrap_or_default()
    });
    let mut stmt = db.prepare(
        "SELECT snapshot_ts, repo, open_issues, age_lt7d, age_7_30d,
                age_30_90d, age_gt90d, median_age_days
         FROM backlog_health WHERE snapshot_ts = ?1
         ORDER BY open_issues DESC"
    ).unwrap();
    let rows: Vec<BacklogRow> = stmt.query_map(params![ts], |r| {
        Ok(BacklogRow {
            snapshot_ts: r.get(0)?, repo: r.get(1)?,
            open_issues: r.get(2).unwrap_or(0),
            age_lt7d: r.get(3).unwrap_or(0), age_7_30d: r.get(4).unwrap_or(0),
            age_30_90d: r.get(5).unwrap_or(0), age_gt90d: r.get(6).unwrap_or(0),
            median_age_days: r.get(7).unwrap_or(0.0),
        })
    }).unwrap().filter_map(|r| r.ok()).collect();
    Json(rows)
}

// ── /api/commits/weekly ───────────────────────────────────────────────────────

#[derive(Serialize)]
struct WeeklyCommitRow {
    snapshot_ts: String,
    repo: String,
    week_offset: i64,
    week_label: String,
    commits: i64,
}

async fn api_weekly_commits(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SnapshotQuery>,
) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    let ts = q.snapshot_ts.unwrap_or_else(|| {
        db.query_row("SELECT MAX(snapshot_ts) FROM weekly_commits", [], |r| r.get(0))
            .unwrap_or_default()
    });

    let filter_repo = q.repo.unwrap_or_default();
    let rows: Vec<WeeklyCommitRow> = if filter_repo.is_empty() {
        // Aggregate across all repos for the snapshot
        let mut stmt = db.prepare(
            "SELECT snapshot_ts, 'ALL' as repo, week_offset, week_label, SUM(commits)
             FROM weekly_commits WHERE snapshot_ts = ?1
             GROUP BY week_offset ORDER BY week_offset DESC"
        ).unwrap();
        stmt.query_map(params![ts], |r| {
            Ok(WeeklyCommitRow {
                snapshot_ts: r.get(0)?, repo: r.get(1)?,
                week_offset: r.get(2)?, week_label: r.get(3)?,
                commits: r.get(4).unwrap_or(0),
            })
        }).unwrap().filter_map(|r| r.ok()).collect()
    } else {
        let mut stmt = db.prepare(
            "SELECT snapshot_ts, repo, week_offset, week_label, commits
             FROM weekly_commits WHERE snapshot_ts = ?1 AND repo = ?2
             ORDER BY week_offset DESC"
        ).unwrap();
        stmt.query_map(params![ts, filter_repo], |r| {
            Ok(WeeklyCommitRow {
                snapshot_ts: r.get(0)?, repo: r.get(1)?,
                week_offset: r.get(2)?, week_label: r.get(3)?,
                commits: r.get(4).unwrap_or(0),
            })
        }).unwrap().filter_map(|r| r.ok()).collect()
    };
    Json(rows)
}

// ── /api/contributors ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ContribRow {
    snapshot_ts: String,
    repo: String,
    login: String,
    contributions: i64,
    share_pct: f64,
}

async fn api_contributors(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SnapshotQuery>,
) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    let ts = q.snapshot_ts.unwrap_or_else(|| {
        db.query_row("SELECT MAX(snapshot_ts) FROM contributors", [], |r| r.get(0))
            .unwrap_or_default()
    });
    let limit = q.limit.unwrap_or(20);
    let mut stmt = db.prepare(
        "SELECT snapshot_ts, repo, login, contributions, share_pct
         FROM contributors WHERE snapshot_ts = ?1
         ORDER BY contributions DESC LIMIT ?2"
    ).unwrap();
    let rows: Vec<ContribRow> = stmt.query_map(params![ts, limit], |r| {
        Ok(ContribRow {
            snapshot_ts: r.get(0)?, repo: r.get(1)?,
            login: r.get(2)?, contributions: r.get(3).unwrap_or(0),
            share_pct: r.get(4).unwrap_or(0.0),
        })
    }).unwrap().filter_map(|r| r.ok()).collect();
    Json(rows)
}

// ── /api/pulls ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct PullRow {
    snapshot_ts: String,
    repo: String,
    number: i64,
    title: String,
    author: Option<String>,
    state: Option<String>,
    updated_at: Option<String>,
    html_url: Option<String>,
}

async fn api_pulls(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SnapshotQuery>,
) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    let ts = q.snapshot_ts.unwrap_or_else(|| {
        db.query_row("SELECT MAX(snapshot_ts) FROM pull_requests", [], |r| r.get(0))
            .unwrap_or_default()
    });
    let limit = q.limit.unwrap_or(30);
    let mut stmt = db.prepare(
        "SELECT snapshot_ts, repo, number, title, author, state, updated_at, html_url
         FROM pull_requests WHERE snapshot_ts = ?1
         ORDER BY updated_at DESC LIMIT ?2"
    ).unwrap();
    let rows: Vec<PullRow> = stmt.query_map(params![ts, limit], |r| {
        Ok(PullRow {
            snapshot_ts: r.get(0)?, repo: r.get(1)?,
            number: r.get(2)?, title: r.get(3)?,
            author: r.get(4)?, state: r.get(5)?,
            updated_at: r.get(6)?, html_url: r.get(7)?,
        })
    }).unwrap().filter_map(|r| r.ok()).collect();
    Json(rows)
}

// ── /api/groups ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct GroupSummary {
    group: String,
    repo_count: i64,
    total_stars: i64,
    total_forks: i64,
    total_open_issues: i64,
    avg_median_age: f64,
    total_issues_opened: i64,
    total_prs_opened: i64,
}

async fn api_groups(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SnapshotQuery>,
) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    let ts = q.snapshot_ts.unwrap_or_else(|| {
        db.query_row("SELECT MAX(snapshot_ts) FROM repo_snapshots", [], |r| r.get(0))
            .unwrap_or_default()
    });
    let mut stmt = db.prepare(
        "SELECT r.repo_group,
                COUNT(DISTINCT r.repo),
                SUM(r.stars),
                SUM(r.forks),
                SUM(r.open_issues_api),
                COALESCE(AVG(b.median_age_days), 0.0),
                COALESCE(SUM(a.issues_opened), 0),
                COALESCE(SUM(a.prs_opened), 0)
         FROM repo_snapshots r
         LEFT JOIN backlog_health b ON b.repo = r.repo AND b.snapshot_ts = r.snapshot_ts
         LEFT JOIN activity_windows a ON a.repo = r.repo AND a.snapshot_ts = r.snapshot_ts
         WHERE r.snapshot_ts = ?1
         GROUP BY r.repo_group ORDER BY r.repo_group"
    ).unwrap();
    let rows: Vec<GroupSummary> = stmt.query_map(params![ts], |r| {
        Ok(GroupSummary {
            group: r.get(0)?,
            repo_count: r.get(1).unwrap_or(0),
            total_stars: r.get(2).unwrap_or(0),
            total_forks: r.get(3).unwrap_or(0),
            total_open_issues: r.get(4).unwrap_or(0),
            avg_median_age: r.get(5).unwrap_or(0.0),
            total_issues_opened: r.get(6).unwrap_or(0),
            total_prs_opened: r.get(7).unwrap_or(0),
        })
    }).unwrap().filter_map(|r| r.ok()).collect();
    Json(rows)
}

// ── /api/momentum ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct MomentumRow {
    repo: String,
    repo_group: String,
    stars: i64,
    net_delta: i64,
    issues_opened: i64,
    issues_closed: i64,
    prs_opened: i64,
    prs_closed: i64,
    open_issues: i64,
    median_age: f64,
}

async fn api_momentum(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SnapshotQuery>,
) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    let ts = q.snapshot_ts.unwrap_or_else(|| {
        db.query_row("SELECT MAX(snapshot_ts) FROM repo_snapshots", [], |r| r.get(0))
            .unwrap_or_default()
    });
    let mut stmt = db.prepare(
        "SELECT r.repo, r.repo_group, r.stars,
                COALESCE(a.net_issues_delta,0),
                COALESCE(a.issues_opened,0), COALESCE(a.issues_closed,0),
                COALESCE(a.prs_opened,0), COALESCE(a.prs_closed,0),
                COALESCE(b.open_issues,0), COALESCE(b.median_age_days,0.0)
         FROM repo_snapshots r
         LEFT JOIN activity_windows a ON a.repo=r.repo AND a.snapshot_ts=r.snapshot_ts
         LEFT JOIN backlog_health b ON b.repo=r.repo AND b.snapshot_ts=r.snapshot_ts
         WHERE r.snapshot_ts = ?1
         ORDER BY a.net_issues_delta ASC"
    ).unwrap();
    let rows: Vec<MomentumRow> = stmt.query_map(params![ts], |r| {
        Ok(MomentumRow {
            repo: r.get(0)?, repo_group: r.get(1)?,
            stars: r.get(2).unwrap_or(0),
            net_delta: r.get(3).unwrap_or(0),
            issues_opened: r.get(4).unwrap_or(0), issues_closed: r.get(5).unwrap_or(0),
            prs_opened: r.get(6).unwrap_or(0), prs_closed: r.get(7).unwrap_or(0),
            open_issues: r.get(8).unwrap_or(0), median_age: r.get(9).unwrap_or(0.0),
        })
    }).unwrap().filter_map(|r| r.ok()).collect();
    Json(rows)
}

// ── /api/imported-files ───────────────────────────────────────────────────────

#[derive(Serialize)]
struct ImportedFile {
    filename: String,
    sha256: String,
    imported_at: String,
}

async fn api_imported_files(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    let mut stmt = db.prepare(
        "SELECT filename, sha256, imported_at FROM _imported_files ORDER BY imported_at DESC"
    ).unwrap();
    let rows: Vec<ImportedFile> = stmt.query_map([], |r| {
        Ok(ImportedFile {
            filename: r.get(0)?, sha256: r.get(1)?, imported_at: r.get(2)?,
        })
    }).unwrap().filter_map(|r| r.ok()).collect();
    Json(rows)
}

// ── Embedded UI ───────────────────────────────────────────────────────────────

const UI_HTML: &str = include_str!("ui.html");
