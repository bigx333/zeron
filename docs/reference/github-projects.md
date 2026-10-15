# Create a project from GitHub

Open **New project**, choose the device that should hold the checkout, then select **Clone from GitHub** alongside Home and the device's drives.

Zeron lists repositories available to that device's active GitHub CLI account, including private repositories and repositories shared through collaborators or organizations. Recent repositories appear first; more pages load in the background. Search by repository or owner name. Select a repository or press Enter to clone it and open the new project. The footer's **Clone project** button and Cmd/Ctrl+Enter do the same.

The selected device needs Git, [GitHub CLI](https://cli.github.com/), and an account connected with [`gh auth login`](https://cli.github.com/manual/gh_auth_login). Missing CLI or authentication errors include setup guidance and a Retry button. Remote devices use their own account and credentials.

Clones are stored at `<engine data directory>/repos/github.com/<owner>/<repository>`. Different owners can have repositories with the same name. A retry reuses a checkout only when its GitHub origin matches and Zeron recorded a successful clone. Incomplete checkouts are preserved and reported with recovery guidance; unrelated folders are preserved and reported as conflicts. An existing project for that checkout is selected without creating a duplicate, including when another client creates it concurrently.

Cloning continues if the picker is dismissed. Completion adds the project without navigating away from a newly opened picker. Cloning and repository listing never require entering a token in Zeron.

The device RPCs are `ListGitHubRepos { cursor? }` and `CloneGitHubRepo { repository }`, both supporting `targetDeviceId`. Listing returns `login`, `repositories` (`nameWithOwner`, `description`, `isPrivate`), and `nextCursor`. Cloning returns the normal `Repo` response; project creation uses the existing `createSpace` workspace mutation, which now also returns the authoritative `space` row. Older coordinators are supported through `WatchSpaces`. Clones of different repositories can run concurrently; waiting and cloning share a 14-minute budget within the 15-minute relay deadline.
