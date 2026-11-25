//! PR review pipeline: GitHub fetch, detectors, comments, and persistence.

mod findings;
pub(crate) mod github;
mod llm;
mod persist;
mod pipeline;
mod reviewers;

pub use github::fetch_pull_request;
pub(crate) use github::{
    fetch_pr_files, github_api_headers, github_get, github_post, next_github_link,
    post_or_update_comment, unified_diff_from_files, GITHUB_CLIENT,
};
pub use pipeline::{review_pr_with_options, ReviewOptions};
