//! Identity for managed GitHub checkouts, shared by every creating client.

/// Independent coordinators can create a project before seeing each other's
/// registry writes. Use the host's returned checkout path so they write the
/// same row instead of creating competing randomly named projects.
pub fn github_project_space_id(device_id: &str, checkout_path: &str) -> String {
    let identity = serde_json::to_vec(&("zeron:github-project:v1", device_id, checkout_path))
        .expect("strings serialize");
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, &identity).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkout_identity_is_shared_and_scoped_to_its_host_and_path() {
        let id = github_project_space_id("host", "/managed/repo");
        assert_eq!(id, github_project_space_id("host", "/managed/repo"));
        assert_ne!(id, github_project_space_id("other-host", "/managed/repo"));
        assert_ne!(id, github_project_space_id("host", "/other/repo"));
        assert_ne!(
            github_project_space_id("ab", "c"),
            github_project_space_id("a", "bc")
        );
    }
}
