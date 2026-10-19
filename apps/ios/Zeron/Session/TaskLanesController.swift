import UIKit

// The phone's lanes card: what the open chat delegated (tasks, with their
// true status, role, checkout, result and whether it was sent back here),
// its side chats and forks, and the way back up to the chat it came from.
// A sheet over the session, so the transcript stays where it was.
//
// Every status word comes from the task record the core projects
// (`SessionRow.task`); the only local state is a request in flight until the
// next workspace frame confirms it.

// MARK: Model

/// How a task reads at a glance — its mark.
enum TaskMark: Equatable {
    case queued, working, waitingOnTasks, needsInput, stopping
    /// Completed, result not yet sent to the parent.
    case ready
    /// Completed and sent (or stopped, which suppresses delivery).
    case sent
    case failed, cancelled, interrupted

    var terminal: Bool {
        switch self {
        case .ready, .sent, .failed, .cancelled, .interrupted: true
        default: false
        }
    }
}

extension DelegatedTaskView {
    /// `requested`: a stop this phone asked for that no frame shows yet.
    func mark(requested: Bool = false) -> TaskMark {
        if (stopping || cancelRequested || requested) && !terminal { return .stopping }
        switch status {
        case "queued": return .queued
        case "running": return .working
        case "waiting": return .needsInput
        case "waiting_for_children": return .waitingOnTasks
        case "completed": return delivered || cancelRequested ? .sent : .ready
        case "failed": return .failed
        case "cancelled": return .cancelled
        case "interrupted": return .interrupted
        default: return terminal ? .sent : .working
        }
    }

    /// Ended with nothing left owed here: sent, or stopped (a stop
    /// suppresses delivery). These fold away.
    var settled: Bool { terminal && (delivered || cancelRequested) }

    func statusText(requested: Bool = false) -> String {
        mark(requested: requested) == .stopping ? "Stopping…" : statusLabel
    }

    /// Where the result stands once there is one. `delivered` means the
    /// parent's result command applied — it reads "sent", never "read".
    var deliveryText: String? {
        guard terminal else { return nil }
        if delivered { return "Sent to this chat" }
        if cancelRequested { return "Stopped from this chat" }
        return "Result ready, not sent yet"
    }

    func outcomeNote(requested: Bool = false) -> String? {
        let hasSummary = !(summary ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        switch mark(requested: requested) {
        case .queued: return "Waiting to start."
        case .needsInput: return "It's waiting on an answer. Open it to reply."
        case .waitingOnTasks: return "Waiting for its own tasks to finish."
        case .stopping: return "Stopping its current turn."
        case .failed where !hasSummary: return "It stopped without a result."
        case .interrupted where !hasSummary: return "It was interrupted before finishing."
        case .cancelled where !hasSummary: return "Cancelled before it reported a result."
        case .ready where !hasSummary, .sent where !hasSummary: return "Finished without a summary."
        default: return nil
        }
    }

    /// "for 4m" while it runs, "4m ago" once it ended.
    func elapsed(now: Date = Date()) -> String {
        let ms = terminal ? updatedAtMs : createdAtMs
        let secs = max(0, Int(now.timeIntervalSince1970 - Double(ms) / 1000))
        let short = secs < 60 ? nil : secs < 3600 ? "\(secs / 60)m" : secs < 86400 ? "\(secs / 3600)h" : "\(secs / 86400)d"
        if terminal { return short.map { "\($0) ago" } ?? "just now" }
        return short.map { "for \($0)" } ?? "just started"
    }
}

/// The task lane: what still owes this chat something rides it, the rest
/// folds away.
struct TaskLane {
    /// Unfinished, or finished with the result not sent yet; oldest first,
    /// so the longest-running leads and new tasks join at the foot.
    var active: [SessionRow] = []
    /// Sent or stopped, most recently updated first.
    var finished: [SessionRow] = []

    init(_ children: [SessionRow]) {
        let tasks = children.filter { $0.task != nil && !$0.archived }
        active = tasks.filter { !$0.task!.settled }.sorted { ($0.task!.createdAtMs, $0.id) < ($1.task!.createdAtMs, $1.id) }
        finished = tasks.filter { $0.task!.settled }.sorted { $0.task!.updatedAtMs > $1.task!.updatedAtMs }
    }

    var isEmpty: Bool { active.isEmpty && finished.isEmpty }
}

extension AppModel {
    /// The chat's live children (tasks and side chats), recency order.
    func children(of chatId: String) -> [SessionRow] {
        (client?.childSessions(parentId: chatId) ?? []).filter { !$0.archived }
    }

    func cancelTask(_ task: DelegatedTaskView) async throws {
        guard let client else { throw CoreError.Closed }
        try await client.cancelTask(parentChatId: task.parentChatId, taskId: task.id)
    }

    /// A fork of the chat through its latest response; its new chat id.
    func forkSession(_ sourceChatId: String) async throws -> String {
        guard let client else { throw CoreError.Closed }
        return try await client.forkSession(sourceChatId: sourceChatId)
    }

    @discardableResult
    func mergeChat(_ sourceChatId: String) async throws -> String {
        guard let client else { throw CoreError.Closed }
        return try await client.mergeChat(sourceChatId: sourceChatId)
    }

    /// Where `row` came from: the chat that delegated it, or a fork's source.
    func lineage(of row: SessionRow) -> (label: String, target: String)? {
        if let task = row.task { return ("Delegated by", task.parentChatId) }
        if let fork = row.fork { return ("Forked from", fork.sourceChatId) }
        return nil
    }

}

// MARK: Type

/// The app's faces through Dynamic Type: scaled from the body style and
/// tracking the setting live on labels that opt in.
enum LaneType {
    static func font(_ face: FaceRole, _ size: CGFloat, _ style: UIFont.TextStyle = .body) -> UIFont {
        UIFontMetrics(forTextStyle: style).scaledFont(for: Fonts.ui(face, size), maximumPointSize: size * 2)
    }

    static func label(_ face: FaceRole, _ size: CGFloat, _ style: UIFont.TextStyle = .body) -> UILabel {
        let label = UILabel()
        apply(label, face, size, style)
        return label
    }

    static func apply(_ label: UILabel, _ face: FaceRole, _ size: CGFloat, _ style: UIFont.TextStyle = .body) {
        label.font = font(face, size, style)
        label.adjustsFontForContentSizeCategory = true
    }
}

// MARK: Motion

enum LaneMotion {
    static var reduced: Bool { UIAccessibility.isReduceMotionEnabled }

    /// Disclosure: a soft spring that lands without overshoot; reduced
    /// motion keeps only a short crossfade.
    static func disclose(_ changes: @escaping () -> Void, completion: ((Bool) -> Void)? = nil) {
        if reduced {
            UIView.animate(withDuration: 0.18, delay: 0, options: [.curveEaseOut, .allowUserInteraction, .beginFromCurrentState], animations: changes, completion: completion)
        } else {
            UIView.animate(withDuration: 0.42, delay: 0, usingSpringWithDamping: 0.88, initialSpringVelocity: 0, options: [.allowUserInteraction, .beginFromCurrentState], animations: changes, completion: completion)
        }
    }
}

// MARK: Sheet

/// Presented from a session: the lineage row, Tasks, Side chats.
final class TaskLanesController: UIViewController {
    private let app: AppModel
    private let chatId: String
    /// Navigate into a chat (a child, or the parent) from the session.
    private let open: (String) -> Void
    private let scroll = UIScrollView()
    private let stack = UIStackView()
    private var token: AnyObject?
    private var taskRows: [String: TaskRowView] = [:]
    private var sideRows: [String: SideChatRowView] = [:]
    private var expanded: Set<String> = []
    private var finishedOpen = false
    /// Stops in flight, then asked for until a frame shows them.
    private var stopping: Set<String> = []
    private var requested: Set<String> = []
    private var errors: [String: String] = [:]
    private var merging: Set<String> = []
    private var merged: Set<String> = []
    private var marks: [String: TaskMark] = [:]
    private var forking = false
    // Rows that persist across frames, so VoiceOver focus and layout stay put.
    private let lineageSeparator = SeparatorView()
    private let tasksSeparator = SeparatorView()
    private var lineageTarget: String?
    private lazy var lineageRow: LineageRowView = {
        let row = LineageRowView()
        row.addAction(UIAction { [weak self] _ in
            guard let self, let target = self.lineageTarget else { return }
            self.navigate(to: target)
        }, for: .touchUpInside)
        return row
    }()
    private let historySeparator = SeparatorView()
    private lazy var history = HistorySection(app: app, chatId: chatId, host: self) { [weak self] in
        self?.reload(animated: true)
    }
    private let sideHeader = LaneHeaderView("Side chats")
    private let emptyView = EmptyLanesView()
    private lazy var forkRow: ForkRowView = {
        let row = ForkRowView()
        row.addAction(UIAction { [weak self] _ in self?.fork() }, for: .touchUpInside)
        return row
    }()
    private lazy var fold: FoldRowView = {
        let fold = FoldRowView()
        fold.addAction(UIAction { [weak self] _ in self?.toggleFinished() }, for: .touchUpInside)
        return fold
    }()

    init(app: AppModel, chatId: String, open: @escaping (String) -> Void) {
        self.app = app
        self.chatId = chatId
        self.open = open
        super.init(nibName: nil, bundle: nil)
        modalPresentationStyle = .pageSheet
        if let sheet = sheetPresentationController {
            sheet.detents = [.medium(), .large()]
            sheet.prefersGrabberVisible = true
            sheet.prefersScrollingExpandsWhenScrolledToEdge = true
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        scroll.alwaysBounceVertical = true
        scroll.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(scroll)
        stack.axis = .vertical
        stack.spacing = 2
        stack.translatesAutoresizingMaskIntoConstraints = false
        scroll.addSubview(stack)
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: view.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            scroll.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            stack.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: 22),
            stack.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -24),
            stack.leadingAnchor.constraint(equalTo: scroll.frameLayoutGuide.leadingAnchor, constant: 12),
            stack.trailingAnchor.constraint(equalTo: scroll.frameLayoutGuide.trailingAnchor, constant: -12),
        ])
        token = app.observeWorkspace { [weak self] in self?.reload(animated: true) }
        // Attributed status lines carry fixed fonts: rebuild them when the
        // text size changes.
        registerForTraitChanges([UITraitPreferredContentSizeCategory.self]) { (self: Self, _: UITraitCollection) in
            self.reload(animated: false)
        }
        reload(animated: false)
    }

    // MARK: Building

    private func reload(animated: Bool) {
        let row = app.row(chatId)
        let children = app.children(of: chatId)
        let lane = TaskLane(children)
        let sides = children.filter { $0.task == nil }
        for row in lane.active + lane.finished {
            guard let task = row.task else { continue }
            if task.cancelRequested || task.terminal { requested.remove(row.id) }
            if task.terminal { errors[row.id] = nil }
        }

        var views: [UIView] = []
        if let row, let lineage = app.lineage(of: row) {
            lineageRow.configure(label: lineage.label, title: app.row(lineage.target)?.title)
            lineageTarget = lineage.target
            views.append(lineageRow)
            views.append(lineageSeparator)
        }
        if !lane.isEmpty {
            views.append(LaneHeaderView("Tasks"))
            for row in lane.active { views.append(taskRow(row, animated: animated)) }
            if !lane.finished.isEmpty {
                fold.set(count: lane.finished.count, open: finishedOpen)
                views.append(fold)
                if finishedOpen { for row in lane.finished { views.append(taskRow(row, animated: animated)) } }
            }
        }
        if lane.isEmpty && sides.isEmpty {
            views.append(emptyView)
        }
        if !lane.isEmpty { views.append(tasksSeparator) }
        views.append(sideHeader)
        for side in sides { views.append(sideRow(side, parentDevice: row?.deviceId)) }
        // Forking is always a tap away, so the first fork is discoverable.
        forkRow.busy = forking
        views.append(forkRow)
        // History: the checkpoints each finished turn left, restorable.
        let past = history.views(chat: row, animated: animated)
        if !past.isEmpty {
            views.append(historySeparator)
            views += past
        }

        // A frame that changes nothing restacks nothing: no relayout spring
        // on every workspace tick.
        let unchanged = stack.arrangedSubviews.count == views.count
            && zip(stack.arrangedSubviews, views).allSatisfy { $0 === $1 }
        if unchanged {
            return
        }
        let apply = {
            for old in self.stack.arrangedSubviews where !views.contains(where: { $0 === old }) {
                self.stack.removeArrangedSubview(old)
                old.removeFromSuperview()
            }
            for (index, view) in views.enumerated() {
                if self.stack.arrangedSubviews.firstIndex(of: view) != index {
                    self.stack.insertArrangedSubview(view, at: index)
                }
            }
            self.stack.layoutIfNeeded()
        }
        if animated, view.window != nil { LaneMotion.disclose(apply) } else { apply() }
    }

    private func taskRow(_ row: SessionRow, animated: Bool) -> TaskRowView {
        let view = taskRows[row.id] ?? {
            let view = TaskRowView()
            view.onToggle = { [weak self] in self?.toggle(row.id) }
            view.onOpen = { [weak self] in self?.navigate(to: row.id) }
            view.onStop = { [weak self, weak view] in
                guard let self, let view else { return }
                self.confirmStop(row.id, from: view.stopButton)
            }
            taskRows[row.id] = view
            return view
        }()
        guard let task = row.task else { return view }
        let mark = task.mark(requested: requested.contains(row.id))
        let previous = marks[row.id]
        marks[row.id] = mark
        view.configure(
            row: row,
            task: task,
            mark: mark,
            requested: requested.contains(row.id),
            inFlight: stopping.contains(row.id),
            error: errors[row.id],
            expanded: expanded.contains(row.id),
            animated: animated
        )
        // A task reaching its end while watched washes once — never on
        // first sight, and Ready → Sent is not a second ending.
        if animated, let previous, !previous.terminal, mark.terminal {
            view.settle(mark)
        }
        return view
    }

    private func sideRow(_ row: SessionRow, parentDevice: String?) -> SideChatRowView {
        let view = sideRows[row.id] ?? {
            let view = SideChatRowView()
            view.onOpen = { [weak self] in self?.navigate(to: row.id) }
            view.onMerge = { [weak self, weak view] in
                guard let self, let view else { return }
                self.confirmMerge(row.id, from: view.mergeButton)
            }
            sideRows[row.id] = view
            return view
        }()
        let busy = row.indicator == .working || row.indicator == .awaitingInput
        view.configure(
            row: row,
            canMerge: row.fork != nil && row.deviceId == parentDevice,
            busy: busy,
            merging: merging.contains(row.id),
            merged: merged.contains(row.id)
        )
        return view
    }

    // MARK: Actions

    private func toggle(_ id: String) {
        if !expanded.insert(id).inserted { expanded.remove(id) }
        reload(animated: true)
    }

    private func toggleFinished() {
        finishedOpen.toggle()
        reload(animated: true)
    }

    private func navigate(to id: String) {
        dismiss(animated: true) { [open] in open(id) }
    }

    private func confirmStop(_ id: String, from anchor: UIView) {
        guard let task = app.row(id)?.task else { return }
        let sheet = UIAlertController(title: "Stop this task?", message: "This stops the task and the tasks it started. Forks and side chats keep running. A stopped task's result isn't sent here.", preferredStyle: .actionSheet)
        sheet.addAction(UIAlertAction(title: "Stop", style: .destructive) { [weak self] _ in self?.stop(id, task) })
        sheet.addAction(UIAlertAction(title: "Keep running", style: .cancel))
        sheet.popoverPresentationController?.sourceView = anchor
        sheet.popoverPresentationController?.sourceRect = anchor.bounds
        present(sheet, animated: true)
    }

    private func stop(_ id: String, _ task: DelegatedTaskView) {
        guard stopping.insert(id).inserted else { return }
        errors[id] = nil
        reload(animated: true)
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                try await self.app.cancelTask(task)
                self.requested.insert(id)
            } catch {
                self.errors[id] = "Couldn't stop the task. " + (error.hostReason ?? "Try again.")
                UIAccessibility.post(notification: .announcement, argument: self.errors[id])
            }
            self.stopping.remove(id)
            self.reload(animated: true)
        }
    }

    /// Fork this chat through its latest response into a side chat, then
    /// go there.
    private func fork() {
        guard !forking else { return }
        forking = true
        reload(animated: true)
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                let id = try await self.app.forkSession(self.chatId)
                self.forking = false
                self.navigate(to: id)
            } catch {
                self.forking = false
                self.reload(animated: true)
                Toast.show("Couldn't fork this chat. " + (error.hostReason ?? "Try again."), in: self.view.window)
            }
        }
    }

    private func confirmMerge(_ id: String, from anchor: UIView) {
        let sheet = UIAlertController(title: "Merge into this chat?", message: "Sends the fork's latest response here as context.", preferredStyle: .actionSheet)
        sheet.addAction(UIAlertAction(title: "Merge", style: .default) { [weak self] _ in self?.merge(id) })
        sheet.addAction(UIAlertAction(title: "Not now", style: .cancel))
        sheet.popoverPresentationController?.sourceView = anchor
        sheet.popoverPresentationController?.sourceRect = anchor.bounds
        present(sheet, animated: true)
    }

    private func merge(_ id: String) {
        guard merging.insert(id).inserted else { return }
        reload(animated: true)
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                try await self.app.mergeChat(id)
                self.merged.insert(id)
                UINotificationFeedbackGenerator().notificationOccurred(.success)
                Toast.show("Merged into this chat", in: self.view.window)
            } catch {
                Toast.show("Couldn't merge. " + (error.hostReason ?? "Try again."), in: self.view.window)
            }
            self.merging.remove(id)
            self.reload(animated: true)
        }
    }
}

// MARK: Rows

/// Small sentence-case section label, the sidebar's header voice.
private final class LaneHeaderView: UIView {
    init(_ text: String) {
        super.init(frame: .zero)
        let label = UILabel()
        label.text = text
        LaneType.apply(label, .sansMedium, 13)
        label.textColor = Palette.secondary
        label.accessibilityTraits = .header
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        NSLayoutConstraint.activate([
            label.topAnchor.constraint(equalTo: topAnchor, constant: 10),
            label.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -4),
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            label.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -12),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }
}

private final class SeparatorView: UIView {
    override init(frame: CGRect) {
        super.init(frame: frame)
        let line = UIView()
        line.backgroundColor = Palette.hairline
        line.translatesAutoresizingMaskIntoConstraints = false
        addSubview(line)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 13),
            line.heightAnchor.constraint(equalToConstant: 1 / UIScreen.main.scale),
            line.centerYAnchor.constraint(equalTo: centerYAnchor),
            line.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            line.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }
}

/// An empty sheet is an invitation, not a dead end.
private final class EmptyLanesView: UIView {
    override init(frame: CGRect) {
        super.init(frame: frame)
        let title = UILabel()
        title.text = "Nothing delegated yet"
        LaneType.apply(title, .sansMedium, 15)
        title.textColor = Palette.text
        let body = UILabel()
        body.text = "Tasks an agent hands off from this chat show up here. Fork it to try another direction."
        LaneType.apply(body, .sans, 13)
        body.textColor = Palette.secondary
        body.numberOfLines = 0
        let column = UIStackView(arrangedSubviews: [title, body])
        column.axis = .vertical
        column.spacing = 4
        column.translatesAutoresizingMaskIntoConstraints = false
        addSubview(column)
        NSLayoutConstraint.activate([
            column.topAnchor.constraint(equalTo: topAnchor, constant: 12),
            column.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -12),
            column.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            column.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }
}

/// A pressable 48pt row with the session list's highlight.
class LaneRowControl: UIControl {
    override init(frame: CGRect) {
        super.init(frame: frame)
        layer.cornerRadius = 12
        layer.cornerCurve = .continuous
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isHighlighted: Bool {
        didSet {
            guard isHighlighted != oldValue else { return }
            UIView.animate(withDuration: isHighlighted ? 0.08 : 0.22, delay: 0, options: [.allowUserInteraction, .beginFromCurrentState]) {
                self.backgroundColor = self.isHighlighted ? Palette.controlFill : .clear
            }
        }
    }
}

/// "Delegated by …" / "Forked from …": the way back up, at any depth.
private final class LineageRowView: LaneRowControl {
    private let lead = LaneType.label(.sans, 15)
    private let name = FadingLabel()

    override init(frame: CGRect) {
        super.init(frame: frame)
        let arrow = UIImageView(image: UIImage(systemName: "chevron.backward", withConfiguration: UIImage.SymbolConfiguration(pointSize: 13, weight: .semibold)))
        arrow.tintColor = Palette.secondary
        arrow.contentMode = .center
        lead.textColor = Palette.secondary
        lead.setContentHuggingPriority(.required, for: .horizontal)
        lead.setContentCompressionResistancePriority(.required, for: .horizontal)
        LaneType.apply(name, .sansMedium, 15)
        let row = UIStackView(arrangedSubviews: [arrow, lead, name])
        row.spacing = 8
        row.alignment = .center
        row.isUserInteractionEnabled = false
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 48),
            arrow.widthAnchor.constraint(equalToConstant: 16),
            row.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            row.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            row.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
        ])
        isAccessibilityElement = true
    }

    required init?(coder: NSCoder) { fatalError() }

    func configure(label: String, title: String?) {
        lead.text = label
        name.text = title ?? "a chat that's no longer here"
        name.textColor = title == nil ? Palette.tertiary : Palette.text
        isEnabled = title != nil
        accessibilityTraits = title == nil ? .staticText : .link
        accessibilityLabel = "\(label) \(title ?? "a chat that's no longer here")"
    }
}

/// "Fork chat": a side chat through the latest response.
private final class ForkRowView: LaneRowControl {
    private let icon = UIImageView(image: UIImage(systemName: "arrow.triangle.branch", withConfiguration: UIImage.SymbolConfiguration(pointSize: 14, weight: .medium)))
    private let label = LaneType.label(.sans, 16)
    private let spinner = UIActivityIndicatorView(style: .medium)

    var busy = false {
        didSet {
            guard busy != oldValue else { return }
            isEnabled = !busy
            label.text = busy ? "Forking…" : "Fork chat"
            accessibilityValue = busy ? "In progress" : nil
            if busy { spinner.startAnimating() } else { spinner.stopAnimating() }
            icon.isHidden = busy
        }
    }

    override init(frame: CGRect) {
        super.init(frame: frame)
        icon.tintColor = Palette.secondary
        icon.contentMode = .center
        spinner.hidesWhenStopped = true
        label.text = "Fork chat"
        label.textColor = Palette.text
        let slot = UIView()
        for v in [icon, spinner] as [UIView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            slot.addSubview(v)
            NSLayoutConstraint.activate([v.centerXAnchor.constraint(equalTo: slot.centerXAnchor), v.centerYAnchor.constraint(equalTo: slot.centerYAnchor)])
        }
        let row = UIStackView(arrangedSubviews: [slot, label])
        row.spacing = 10
        row.alignment = .center
        row.isUserInteractionEnabled = false
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 48),
            slot.widthAnchor.constraint(equalToConstant: 16),
            slot.heightAnchor.constraint(equalToConstant: 16),
            row.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            row.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            row.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -12),
        ])
        isAccessibilityElement = true
        accessibilityTraits = .button
        accessibilityLabel = "Fork chat"
        accessibilityHint = "Starts a side chat from the latest response"
    }

    required init?(coder: NSCoder) { fatalError() }
}

/// The finished fold: "3 finished", its chevron turning as it opens.
final class FoldRowView: LaneRowControl {
    private let chevron = UIImageView(image: UIImage(systemName: "chevron.right", withConfiguration: UIImage.SymbolConfiguration(pointSize: 12, weight: .semibold)))
    private let label = UILabel()
    private var open = false

    override init(frame: CGRect) {
        super.init(frame: frame)
        chevron.tintColor = Palette.tertiary
        chevron.contentMode = .center
        LaneType.apply(label, .sans, 15)
        label.textColor = Palette.secondary
        let row = UIStackView(arrangedSubviews: [chevron, label])
        row.spacing = 10
        row.alignment = .center
        row.isUserInteractionEnabled = false
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
            chevron.widthAnchor.constraint(equalToConstant: 16),
            row.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            row.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            row.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -12),
        ])
        isAccessibilityElement = true
        accessibilityTraits = .button
    }

    required init?(coder: NSCoder) { fatalError() }

    /// `noun`: what the fold holds, when not finished tasks.
    func set(count: Int, open: Bool, noun: String? = nil) {
        if let noun {
            label.text = "\(count) \(noun)\(count == 1 ? "" : "s")"
            accessibilityLabel = label.text
        } else {
            label.text = "\(count) finished"
            accessibilityLabel = "\(count) finished tasks"
        }
        accessibilityValue = open ? "Expanded" : "Collapsed"
        guard open != self.open else { return }
        self.open = open
        let turn = { self.chevron.transform = open ? CGAffineTransform(rotationAngle: .pi / 2) : .identity }
        if window != nil { LaneMotion.disclose(turn) } else { turn() }
    }
}

/// A delegated task: header row (tap to inspect) over its detail.
private final class TaskRowView: UIView {
    var onToggle: (() -> Void)?
    var onOpen: (() -> Void)?
    var onStop: (() -> Void)?

    private let header = LaneRowControl()
    private let glyph = StatusGlyph()
    private let harness = UIImageView()
    private let title = FadingLabel()
    private let time = UILabel()
    private let detail = UIStackView()
    private let status = UILabel()
    private let facts = UILabel()
    private let summary = UILabel()
    private let note = UILabel()
    private let shortened = UILabel()
    private let delivery = UILabel()
    private let error = UILabel()
    private let openButton = UIButton(configuration: .gray())
    let stopButton = UIButton(configuration: .plain())
    private var shownMark: TaskMark?
    private var expanded = false

    override init(frame: CGRect) {
        super.init(frame: frame)
        layer.cornerRadius = 14
        layer.cornerCurve = .continuous
        clipsToBounds = true

        glyph.setContentHuggingPriority(.required, for: .horizontal)
        harness.contentMode = .center
        LaneType.apply(title, .sans, 16)
        title.textColor = Palette.text
        LaneType.apply(time, .sans, 13)
        time.textColor = StatusTone.time
        time.setContentHuggingPriority(.required, for: .horizontal)
        time.setContentCompressionResistancePriority(.required, for: .horizontal)
        let line = UIStackView(arrangedSubviews: [glyph, harness, title, time])
        line.spacing = 10
        line.alignment = .center
        line.isUserInteractionEnabled = false
        line.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(line)
        NSLayoutConstraint.activate([
            header.heightAnchor.constraint(greaterThanOrEqualToConstant: 48),
            glyph.widthAnchor.constraint(equalToConstant: 16),
            glyph.heightAnchor.constraint(equalToConstant: 16),
            harness.widthAnchor.constraint(equalToConstant: 15),
            line.topAnchor.constraint(equalTo: header.topAnchor, constant: 8),
            line.bottomAnchor.constraint(equalTo: header.bottomAnchor, constant: -8),
            line.leadingAnchor.constraint(equalTo: header.leadingAnchor, constant: 12),
            line.trailingAnchor.constraint(equalTo: header.trailingAnchor, constant: -12),
        ])
        header.isAccessibilityElement = true
        header.accessibilityTraits = .button
        header.accessibilityHint = "Double-tap to show details"
        header.addAction(UIAction { [weak self] _ in self?.onToggle?() }, for: .touchUpInside)

        LaneType.apply(status, .sansMedium, 14)
        LaneType.apply(facts, .sans, 13)
        facts.textColor = Palette.secondary
        facts.numberOfLines = 0
        LaneType.apply(summary, .sans, 14)
        summary.numberOfLines = 4
        summary.lineBreakMode = .byTruncatingTail
        LaneType.apply(note, .sans, 14)
        note.textColor = Palette.secondary
        note.numberOfLines = 0
        LaneType.apply(delivery, .sans, 13)
        delivery.textColor = Palette.secondary
        delivery.numberOfLines = 0
        LaneType.apply(error, .sans, 14)
        error.textColor = Palette.danger
        error.numberOfLines = 0

        openButton.configuration?.title = "Open task"
        openButton.configuration?.cornerStyle = .capsule
        openButton.configuration?.buttonSize = .small
        openButton.addAction(UIAction { [weak self] _ in self?.onOpen?() }, for: .touchUpInside)
        stopButton.configuration?.cornerStyle = .capsule
        stopButton.configuration?.buttonSize = .small
        stopButton.addAction(UIAction { [weak self] _ in self?.onStop?() }, for: .touchUpInside)
        let actions = UIStackView(arrangedSubviews: [openButton, UIView(), stopButton])
        actions.alignment = .center
        actions.spacing = 8

        LaneType.apply(shortened, .sans, 13)
        shortened.textColor = Palette.tertiary
        shortened.numberOfLines = 0
        shortened.text = "Summary shortened. Open the task to read the full result."
        for v in [status, facts, summary, shortened, note, delivery, error, actions] { detail.addArrangedSubview(v) }
        detail.axis = .vertical
        detail.spacing = 8
        detail.isLayoutMarginsRelativeArrangement = true
        detail.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 0, leading: 50, bottom: 14, trailing: 12)
        detail.isHidden = true
        detail.alpha = 0

        let column = UIStackView(arrangedSubviews: [header, detail])
        column.axis = .vertical
        column.translatesAutoresizingMaskIntoConstraints = false
        addSubview(column)
        NSLayoutConstraint.activate([
            column.topAnchor.constraint(equalTo: topAnchor),
            column.bottomAnchor.constraint(equalTo: bottomAnchor),
            column.leadingAnchor.constraint(equalTo: leadingAnchor),
            column.trailingAnchor.constraint(equalTo: trailingAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func configure(row: SessionRow, task: DelegatedTaskView, mark: TaskMark, requested: Bool, inFlight: Bool, error message: String?, expanded open: Bool, animated: Bool) {
        title.text = row.title
        time.text = row.timeLabel
        harness.image = BrandMarks.image(forModel: row.model, harness: row.harness, side: 15)
        harness.isHidden = harness.image == nil
        setMark(mark, animated: animated)

        let words = task.statusText(requested: requested)
        let tone = Self.tone(mark)
        let styled = NSMutableAttributedString(string: words, attributes: [.foregroundColor: tone, .font: LaneType.font(.sansMedium, 14)])
        styled.append(NSAttributedString(string: "  \(task.elapsed())", attributes: [.foregroundColor: Palette.tertiary, .font: LaneType.font(.sans, 14)]))
        status.attributedText = styled
        facts.text = [task.roleLabel, task.isolationLabel, row.modelLabel ?? row.model].compactMap { $0 }.joined(separator: "   ")
        let body = (task.summary ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        summary.text = body
        summary.textColor = mark == .failed ? Palette.danger.withAlphaComponent(0.85) : Palette.text
        summary.isHidden = body.isEmpty
        shortened.isHidden = body.isEmpty || !task.summaryTruncated
        note.text = task.outcomeNote(requested: requested)
        note.isHidden = note.text == nil
        if let text = task.deliveryText {
            let symbol = task.settled ? "checkmark" : "circle.fill"
            let image = UIImage(systemName: symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: task.settled ? 11 : 6, weight: .semibold))
            let attachment = NSTextAttachment(image: image!.withTintColor(task.settled ? Palette.secondary : StatusTone.working, renderingMode: .alwaysOriginal))
            let line = NSMutableAttributedString(attachment: attachment)
            line.append(NSAttributedString(string: "  " + text))
            delivery.attributedText = line
            delivery.isHidden = false
        } else {
            delivery.isHidden = true
        }
        self.error.text = message
        self.error.isHidden = message == nil

        // Open leads; Needs input makes it the primary.
        openButton.configuration?.baseForegroundColor = mark == .needsInput ? Palette.accent : Palette.text
        let canStop = !task.terminal && !task.cancelRequested && !requested
        stopButton.isHidden = !(canStop || inFlight)
        stopButton.configuration?.showsActivityIndicator = inFlight
        stopButton.configuration?.title = inFlight ? "Stopping…" : message == nil ? "Stop task" : "Try again"
        stopButton.configuration?.baseForegroundColor = inFlight ? Palette.warning : Palette.danger
        stopButton.isEnabled = !inFlight && row.deviceOnline
        if !row.deviceOnline, canStop {
            stopButton.configuration?.subtitle = "Its device is offline"
        } else {
            stopButton.configuration?.subtitle = nil
        }

        header.accessibilityLabel = [row.title, words, task.roleLabel].compactMap { $0 }.joined(separator: ", ")
        header.accessibilityValue = open ? "Expanded" : "Collapsed"
        guard open != expanded else { return }
        expanded = open
        let change = {
            self.detail.isHidden = !open
            self.detail.alpha = open ? 1 : 0
            self.backgroundColor = open ? Palette.controlFill : .clear
            // Rows below glide with the disclosure.
            self.superview?.layoutIfNeeded()
        }
        if animated, window != nil { LaneMotion.disclose(change) } else { change() }
    }

    /// The mark swaps with a short crossfade when the status moves.
    private func setMark(_ mark: TaskMark, animated: Bool) {
        guard mark != shownMark else { return }
        let kind: StatusGlyph.Kind = switch mark {
        case .queued: .dot(StatusTone.idle)
        case .working: .spinner
        case .waitingOnTasks: .trailer
        case .needsInput: .dot(StatusTone.input)
        case .stopping: .dot(Palette.warning)
        case .ready: .check(StatusTone.done)
        case .sent: .check(Palette.tertiary)
        case .failed: .dot(StatusTone.failed)
        case .cancelled: .dot(Palette.tertiary)
        case .interrupted: .dot(Palette.warning.withAlphaComponent(0.7))
        }
        let swap = shownMark != nil && animated && !LaneMotion.reduced
        shownMark = mark
        if swap {
            UIView.transition(with: glyph, duration: 0.28, options: [.transitionCrossDissolve, .allowUserInteraction]) { self.glyph.kind = kind }
        } else {
            glyph.kind = kind
        }
    }

    /// The one unprompted moment: an ending washes the row once, and a
    /// completion taps the hand.
    func settle(_ mark: TaskMark) {
        if mark == .ready || mark == .sent {
            UINotificationFeedbackGenerator().notificationOccurred(.success)
        }
        guard !LaneMotion.reduced else { return }
        let wash: UIColor = switch mark {
        case .ready, .sent: StatusTone.done.withAlphaComponent(0.14)
        case .failed: Palette.danger.withAlphaComponent(0.12)
        default: Palette.controlFill
        }
        let rest = backgroundColor
        backgroundColor = wash
        UIView.animate(withDuration: 1.1, delay: 0, options: [.curveEaseOut, .allowUserInteraction]) {
            self.backgroundColor = rest
        }
    }

    private static func tone(_ mark: TaskMark) -> UIColor {
        switch mark {
        case .needsInput: Palette.accent
        case .ready: Palette.success
        case .failed: Palette.danger
        case .stopping, .interrupted: Palette.warning
        default: Palette.secondary
        }
    }
}

/// A side chat or fork: tap opens it; a fork carries Merge.
private final class SideChatRowView: LaneRowControl {
    var onOpen: (() -> Void)?
    var onMerge: (() -> Void)?
    private let glyph = StatusGlyph()
    private let harness = UIImageView()
    private let forkMark = UIImageView(image: UIImage(systemName: "arrow.triangle.branch", withConfiguration: UIImage.SymbolConfiguration(pointSize: 12, weight: .medium)))
    private let title = FadingLabel()
    private let time = UILabel()
    let mergeButton = UIButton(configuration: .plain())

    override init(frame: CGRect) {
        super.init(frame: frame)
        harness.contentMode = .center
        forkMark.tintColor = Palette.tertiary
        forkMark.contentMode = .center
        LaneType.apply(title, .sans, 16)
        title.textColor = Palette.text
        LaneType.apply(time, .sans, 13)
        time.textColor = StatusTone.time
        time.setContentHuggingPriority(.required, for: .horizontal)
        time.setContentCompressionResistancePriority(.required, for: .horizontal)
        mergeButton.configuration?.image = UIImage(systemName: "arrow.uturn.backward", withConfiguration: UIImage.SymbolConfiguration(pointSize: 13, weight: .semibold))
        mergeButton.configuration?.baseForegroundColor = Palette.secondary
        mergeButton.accessibilityLabel = "Merge into this chat"
        mergeButton.addAction(UIAction { [weak self] _ in self?.onMerge?() }, for: .touchUpInside)
        let line = UIStackView(arrangedSubviews: [glyph, harness, forkMark, title, mergeButton, time])
        line.spacing = 10
        line.alignment = .center
        line.translatesAutoresizingMaskIntoConstraints = false
        addSubview(line)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 48),
            glyph.widthAnchor.constraint(equalToConstant: 16),
            glyph.heightAnchor.constraint(equalToConstant: 16),
            harness.widthAnchor.constraint(equalToConstant: 15),
            mergeButton.widthAnchor.constraint(equalToConstant: 44),
            mergeButton.heightAnchor.constraint(equalToConstant: 44),
            line.topAnchor.constraint(equalTo: topAnchor, constant: 2),
            line.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -2),
            line.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            line.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
        ])
        for v in [glyph, harness, forkMark, title, time] { v.isUserInteractionEnabled = false }
        addAction(UIAction { [weak self] _ in self?.onOpen?() }, for: .touchUpInside)
        isAccessibilityElement = false
    }

    required init?(coder: NSCoder) { fatalError() }

    func configure(row: SessionRow, canMerge: Bool, busy: Bool, merging: Bool, merged: Bool) {
        title.text = row.title
        time.text = merged ? "Merged" : row.timeLabel
        harness.image = BrandMarks.image(forModel: row.model, harness: row.harness, side: 15)
        harness.isHidden = harness.image == nil
        forkMark.isHidden = row.fork == nil
        glyph.kind = switch row.indicator {
        case .working: .spinner
        case .awaitingInput: .dot(StatusTone.input)
        case .errored: .dot(StatusTone.failed)
        case .completed: .check(StatusTone.done)
        case .idle: .dot(StatusTone.idle)
        }
        mergeButton.isHidden = !canMerge
        mergeButton.isEnabled = !busy && !merging
        mergeButton.configuration?.showsActivityIndicator = merging
        mergeButton.accessibilityHint = busy ? "Finish the fork's turn first" : nil
        // The row reads as one element; Merge stays its own button.
        let open = UIAccessibilityElement(accessibilityContainer: self)
        open.accessibilityLabel = [row.fork == nil ? "Side chat" : "Fork", row.title, row.timeLabel].joined(separator: ", ")
        open.accessibilityTraits = .button
        open.accessibilityFrameInContainerSpace = bounds
        accessibilityElements = canMerge ? [open, mergeButton] : [open]
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        (accessibilityElements?.first as? UIAccessibilityElement)?.accessibilityFrameInContainerSpace = bounds
    }
}
