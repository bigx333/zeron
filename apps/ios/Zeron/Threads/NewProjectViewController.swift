import UIKit

/// Create a project on an execution host, from a folder or its GitHub account.
final class NewProjectViewController: UIViewController, UICollectionViewDelegate, UISearchResultsUpdating {
    private let app: AppModel
    private var device: HostOption?
    private var listing: FolderListing?
    private var listingDevice: String?
    private var path: String?
    private var loading = false
    private var creating = false
    private var loadGeneration = 0
    private var loadTask: Task<Void, Never>?
    private var collectionView: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<Int, String>!
    private let source = UISegmentedControl(items: ["Folder", "GitHub"])
    private let search = UISearchController(searchResultsController: nil)
    private let status = UILabel()
    private let useButton = UIButton(type: .system)
    private let retryButton = UIButton(type: .system)
    private var repositories: [GitHubRepository] = []
    private var githubLogin: String?
    private var githubCursor: String?
    private var githubError: String?
    private var selectedRepository: String?
    private var github: Bool { source.selectedSegmentIndex == 1 }

    /// Called with the authoritative project id, ready for a new session.
    var onCreated: ((String) -> Void)?

    init(app: AppModel) {
        self.app = app
        super.init(nibName: nil, bundle: nil)
        title = "New Project"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.leftBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in
            guard let self, !creating else { return }
            dismiss(animated: true)
        })
        source.selectedSegmentIndex = 0
        source.accessibilityIdentifier = "project-source"
        source.addAction(UIAction { [weak self] _ in self?.changeSource() }, for: .valueChanged)
        source.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(source)
        search.searchResultsUpdater = self
        search.obscuresBackgroundDuringPresentation = false
        search.searchBar.placeholder = "Search repositories"
        search.searchBar.accessibilityIdentifier = "github-repository-search"
        navigationItem.hidesSearchBarWhenScrolling = false
        definesPresentationContext = true

        var config = UICollectionLayoutListConfiguration(appearance: .insetGrouped)
        config.backgroundColor = .clear
        collectionView = UICollectionView(frame: .zero, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: config))
        collectionView.translatesAutoresizingMaskIntoConstraints = false
        collectionView.backgroundColor = .clear
        collectionView.keyboardDismissMode = .onDrag
        collectionView.delegate = self
        collectionView.accessibilityIdentifier = "project-entries"
        view.addSubview(collectionView)
        let reg = UICollectionView.CellRegistration<UICollectionViewListCell, String> { [weak self] cell, _, name in
            guard let self else { return }
            var c = UIListContentConfiguration.subtitleCell()
            c.textProperties.font = Fonts.ui(.sansMedium, 16)
            if github, let repo = repositories.first(where: { $0.nameWithOwner == name }) {
                c.text = repo.nameWithOwner
                c.secondaryText = [repo.isPrivate ? "Private" : nil, repo.description].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
                c.secondaryTextProperties.numberOfLines = 2
                c.image = UIImage(systemName: "arrow.triangle.branch")
                c.imageProperties.tintColor = Palette.accent
                cell.accessories = selectedRepository == name ? [.checkmark()] : []
                cell.accessibilityIdentifier = "github-repo-\(name)"
            } else if let entry = listing?.entries.first(where: { $0.name == name }) {
                c.text = entry.name
                c.image = UIImage(systemName: entry.isRepo ? "folder.badge.gearshape" : "folder")
                c.imageProperties.tintColor = entry.isRepo ? Palette.accent : Palette.secondary
                cell.accessories = [.disclosureIndicator()]
                cell.accessibilityIdentifier = "project-folder-\(name)"
            } else {
                c.text = name == ".." ? "Parent Folder" : name
                c.image = UIImage(systemName: "arrow.turn.left.up")
                cell.accessories = []
                cell.accessibilityIdentifier = "project-parent"
            }
            cell.contentConfiguration = c
            var bg = UIBackgroundConfiguration.listGroupedCell()
            bg.backgroundColor = Palette.elevated
            cell.backgroundConfiguration = bg
        }
        dataSource = UICollectionViewDiffableDataSource(collectionView: collectionView) { cv, path, name in
            cv.dequeueConfiguredReusableCell(using: reg, for: path, item: name)
        }
        var use = UIButton.Configuration.filled()
        use.cornerStyle = .capsule
        use.baseBackgroundColor = Palette.text
        use.baseForegroundColor = Palette.background
        use.contentInsets = NSDirectionalEdgeInsets(top: 14, leading: 20, bottom: 14, trailing: 20)
        useButton.configuration = use
        useButton.addAction(UIAction { [weak self] _ in self?.create() }, for: .touchUpInside)
        retryButton.setTitle("Retry", for: .normal)
        retryButton.accessibilityIdentifier = "github-retry"
        retryButton.isHidden = true
        retryButton.addAction(UIAction { [weak self] _ in self?.loadGitHub(reset: false) }, for: .touchUpInside)
        status.font = Fonts.ui(.sans, 13)
        status.textColor = Palette.secondary
        status.numberOfLines = 0
        status.textAlignment = .center
        let bottom = UIStackView(arrangedSubviews: [status, retryButton, useButton])
        bottom.axis = .vertical
        bottom.spacing = 8
        bottom.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(bottom)
        NSLayoutConstraint.activate([
            source.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 20),
            source.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -20),
            source.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 8),
            collectionView.topAnchor.constraint(equalTo: source.bottomAnchor, constant: 8),
            collectionView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            collectionView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            collectionView.bottomAnchor.constraint(equalTo: bottom.topAnchor, constant: -8),
            bottom.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 20),
            bottom.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -20),
            bottom.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -12),
        ])
        let hosts = app.hostOptions
        device = hosts.first(where: \.online) ?? hosts.first
        navigationItem.rightBarButtonItem = UIBarButtonItem(title: device?.name ?? "No hosts", menu: UIMenu(title: "Device", children: hosts.map { host in
            UIAction(title: host.name, subtitle: host.online ? "Online" : "Offline", image: UIImage(systemName: "desktopcomputer")) { [weak self] _ in
                guard let self, !creating else { return }
                device = host
                navigationItem.rightBarButtonItem?.title = host.name
                if github { loadGitHub(reset: true) } else { load(nil) }
            }
        }))
        navigationItem.rightBarButtonItem?.accessibilityIdentifier = "project-host"
        changeSource()
    }

    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        if isBeingDismissed || navigationController?.isBeingDismissed == true { loadTask?.cancel() }
    }

    private func changeSource() {
        guard !creating else { return }
        loadTask?.cancel()
        loadGeneration += 1
        navigationItem.searchController = github ? search : nil
        retryButton.isHidden = true
        selectedRepository = nil
        useButton.accessibilityIdentifier = github ? "clone-github-project" : "use-folder"
        if github {
            title = "Clone from GitHub"
            loadGitHub(reset: true)
        } else {
            search.isActive = false
            load(nil)
        }
    }

    private func applyItems(_ items: [String]) {
        let previous = Set(dataSource.snapshot().itemIdentifiers)
        var snapshot = NSDiffableDataSourceSnapshot<Int, String>()
        snapshot.appendSections([0])
        snapshot.appendItems(items)
        snapshot.reconfigureItems(items.filter { previous.contains($0) })
        dataSource.apply(snapshot, animatingDifferences: false)
    }

    private func load(_ path: String?) {
        guard let device else { showNoHosts(); return }
        loadTask?.cancel()
        loading = true
        status.text = "Loading…"
        useButton.isEnabled = false
        loadGeneration += 1
        let generation = loadGeneration
        loadTask = Task { @MainActor in
            let listing = await app.listFolders(deviceId: device.id, path: path)
            guard generation == loadGeneration, !Task.isCancelled, !github else { return }
            loading = false
            guard let listing else {
                status.text = device.online ? "Couldn't read that folder." : "\(device.name) is offline."
                useButton.isEnabled = self.listing != nil && listingDevice == device.id
                return
            }
            listingDevice = device.id
            self.listing = listing
            self.path = listing.path
            applyItems((Self.parent(of: listing.path) == nil ? [] : [".."]) + listing.entries.filter(\.isDir).map(\.name))
            let name = (listing.path as NSString).lastPathComponent
            useButton.configuration?.title = "Use “\(name.isEmpty ? listing.path : name)”"
            useButton.isEnabled = true
            status.text = listing.path
            title = name.isEmpty ? "New Project" : name
        }
    }

    private func loadGitHub(reset: Bool) {
        guard let device else { showNoHosts(); return }
        loadTask?.cancel()
        loadGeneration += 1
        let generation = loadGeneration
        if reset {
            repositories = []
            githubLogin = nil
            githubCursor = nil
            selectedRepository = nil
            search.searchBar.text = ""
            updateSearchResults(for: search)
        }
        loading = true
        retryButton.isHidden = true
        githubError = nil
        status.text = "Loading GitHub repositories on \(device.name)…"
        updateCloneButton()
        loadTask = Task { @MainActor in
            var seen = Set<String>()
            do {
                repeat {
                    let page = try await app.githubRepositories(deviceId: device.id, cursor: githubCursor)
                    guard generation == loadGeneration, !Task.isCancelled, github else { return }
                    if let login = githubLogin, login != page.login {
                        repositories = []
                        githubLogin = nil
                        githubCursor = nil
                        selectedRepository = nil
                        updateSearchResults(for: search)
                        throw CoreError.HostError(message: "GitHub account changed. Retry to reload repositories.")
                    }
                    githubLogin = page.login
                    var names = Set(repositories.map(\.nameWithOwner))
                    repositories += page.repositories.filter { names.insert($0.nameWithOwner).inserted }
                    githubCursor = page.nextCursor
                    updateSearchResults(for: search)
                    if let cursor = githubCursor, !seen.insert(cursor).inserted {
                        throw CoreError.HostError(message: "GitHub returned a repeated page. Retry to continue.")
                    }
                } while githubCursor != nil
                loading = false
                updateGitHubStatus()
            } catch {
                guard generation == loadGeneration, !Task.isCancelled else { return }
                loading = false
                githubError = Self.githubFailure(error, host: device.name)
                status.text = githubError
                retryButton.isHidden = false
            }
            updateCloneButton()
        }
    }

    func updateSearchResults(for searchController: UISearchController) {
        guard github, dataSource != nil else { return }
        let query = (searchController.searchBar.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        let matches = repositories.filter { query.isEmpty || $0.nameWithOwner.localizedCaseInsensitiveContains(query) }
        applyItems(matches.map(\.nameWithOwner))
        if !loading && !creating && githubError == nil { updateGitHubStatus(empty: matches.isEmpty) }
    }

    private func updateGitHubStatus(empty: Bool = false) {
        guard !creating else { return }
        guard let device else { return }
        let account = githubLogin.map { "Signed in as \($0) on \(device.name)." } ?? "Uses the GitHub CLI account on \(device.name)."
        status.text = empty ? "\(account) No repositories match." : "\(account) The checkout is saved on that device."
    }

    private func updateCloneButton() {
        guard github else { return }
        useButton.configuration?.title = selectedRepository.map { "Clone \($0.split(separator: "/").last.map(String.init) ?? $0)" } ?? "Clone project"
        useButton.isEnabled = !creating && selectedRepository != nil
    }

    private func showNoHosts() {
        status.text = "No desktop devices yet — open Zeron on a computer to add one."
        useButton.isEnabled = false
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
        collectionView.deselectItem(at: indexPath, animated: true)
        guard !creating, let name = dataSource.itemIdentifier(for: indexPath) else { return }
        if github {
            selectedRepository = name
            search.searchBar.resignFirstResponder()
            updateSearchResults(for: search)
            updateCloneButton()
        } else if !loading, let listing {
            load(name == ".." ? Self.parent(of: listing.path) : (listing.path as NSString).appendingPathComponent(name))
        }
    }

    static func parent(of path: String) -> String? {
        let trimmed = path.hasSuffix("/") && path.count > 1 ? String(path.dropLast()) : path
        guard trimmed != "/", trimmed.contains("/") else { return nil }
        let up = (trimmed as NSString).deletingLastPathComponent
        return up.isEmpty ? "/" : up
    }

    static func githubFailure(_ error: Error, host: String) -> String {
        let message = error.localizedDescription
        if message.localizedCaseInsensitiveContains("not installed") || message.localizedCaseInsensitiveContains("not authenticated") {
            return "\(message) Install GitHub CLI and run gh auth login on \(host), then retry."
        }
        if message.localizedCaseInsensitiveContains("unknown method") || message.localizedCaseInsensitiveContains("not supported") {
            return "Update Zeron on \(host) to use GitHub projects."
        }
        return message
    }

    private func setCreating(_ busy: Bool) {
        creating = busy
        isModalInPresentation = busy
        navigationController?.isModalInPresentation = busy
        source.isEnabled = !busy
        navigationItem.leftBarButtonItem?.isEnabled = !busy
        navigationItem.rightBarButtonItem?.isEnabled = !busy
        retryButton.isEnabled = !busy
        useButton.isEnabled = !busy
        useButton.configuration?.showsActivityIndicator = busy
    }

    private func create() {
        guard !creating, let device else { return }
        if github {
            guard let repository = selectedRepository else { return }
            search.searchBar.resignFirstResponder()
            setCreating(true)
            status.text = "Cloning on \(device.name) and adding the project…"
            Task { @MainActor in
                do {
                    let id = try await app.cloneGithubProject(deviceId: device.id, repository: repository)
                    onCreated?(id)
                    setCreating(false)
                    dismiss(animated: true)
                } catch {
                    setCreating(false)
                    updateCloneButton()
                    status.text = Self.githubFailure(error, host: device.name)
                }
            }
        } else {
            guard let path, listingDevice == device.id else { return }
            let git = listing?.entries.contains { $0.name == ".git" } ?? false
            setCreating(true)
            Task { @MainActor in
                let created = await app.createProject(deviceId: device.id, path: path, gitDetected: git)
                setCreating(false)
                if let created { onCreated?(created); dismiss(animated: true) }
                else { status.text = "Couldn't create the project on \(device.name)." }
            }
        }
    }
}
