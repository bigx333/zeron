import UIKit

/// Codex dictation in the composer's trailing controls, just before Send: a
/// microphone at rest; while recording a red stop capsule counting seconds
/// beside ✕ to discard; a spinner while ChatGPT transcribes. Collapses to
/// nothing when dictation is off or unavailable.
final class ComposerDictationControl: UIView {
    private(set) weak var controller: CodexDictationController?
    private weak var composer: ComposerBar?
    private var token: AnyObject?
    private let cancelButton = UIButton(type: .system)
    private let micButton = UIButton(type: .system)
    /// Hidden: no width, so the text and chips keep their room.
    private var collapse: NSLayoutConstraint!
    private var leadMic: NSLayoutConstraint!
    private var leadCancel: NSLayoutConstraint!
    /// The gap before Send, while anything shows.
    private var gap: NSLayoutConstraint!
    /// A round microphone / spinner; the recording capsule fits its clock.
    private var roundWidth: NSLayoutConstraint!
    private var timer: Timer?
    private var shown: Look = .hidden
    private weak var notice: DictationNotice?

    private enum Look: Equatable {
        case hidden
        case mic(enabled: Bool)
        case recording(seconds: Int)
        case transcribing

        /// Same controls, only the seconds moved.
        func sameKind(as other: Look) -> Bool {
            if case .recording = self, case .recording = other { return true }
            return self == other
        }
    }

    private static let control: CGFloat = 34

    override init(frame: CGRect) {
        super.init(frame: frame)
        translatesAutoresizingMaskIntoConstraints = false

        var cancel = UIButton.Configuration.filled()
        cancel.image = UIImage(systemName: "xmark", withConfiguration: UIImage.SymbolConfiguration(pointSize: 12, weight: .bold))
        cancel.baseForegroundColor = Palette.text
        cancel.baseBackgroundColor = Palette.controlFill
        cancel.cornerStyle = .capsule
        cancel.contentInsets = .zero
        cancelButton.configuration = cancel
        cancelButton.accessibilityLabel = "Cancel dictation"
        cancelButton.accessibilityIdentifier = "composer-dictation-cancel"
        cancelButton.addAction(UIAction { [weak self] _ in self?.cancelTapped() }, for: .touchUpInside)

        micButton.accessibilityIdentifier = "composer-dictation"
        micButton.addAction(UIAction { [weak self] _ in self?.micTapped() }, for: .touchUpInside)
        // The recording capsule is as wide as its clock; the draft gives way.
        micButton.setContentHuggingPriority(UILayoutPriority(999), for: .horizontal)
        micButton.setContentCompressionResistancePriority(UILayoutPriority(999), for: .horizontal)

        for button in [cancelButton, micButton] {
            button.translatesAutoresizingMaskIntoConstraints = false
            button.isHidden = true
            addSubview(button)
        }
        collapse = widthAnchor.constraint(equalToConstant: 0)
        leadMic = micButton.leadingAnchor.constraint(equalTo: leadingAnchor)
        leadCancel = cancelButton.leadingAnchor.constraint(equalTo: leadingAnchor)
        gap = micButton.trailingAnchor.constraint(equalTo: trailingAnchor)
        roundWidth = micButton.widthAnchor.constraint(equalToConstant: Self.control)
        NSLayoutConstraint.activate([
            gap,
            roundWidth,
            collapse,
            micButton.centerYAnchor.constraint(equalTo: centerYAnchor),
            micButton.heightAnchor.constraint(equalToConstant: Self.control),
            micButton.widthAnchor.constraint(greaterThanOrEqualToConstant: Self.control),
            cancelButton.trailingAnchor.constraint(equalTo: micButton.leadingAnchor, constant: -6),
            cancelButton.centerYAnchor.constraint(equalTo: centerYAnchor),
            cancelButton.widthAnchor.constraint(equalToConstant: Self.control),
            cancelButton.heightAnchor.constraint(equalToConstant: Self.control),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func attach(_ controller: CodexDictationController?, to composer: ComposerBar) {
        self.composer = composer
        self.controller = controller
        token = controller?.observe { [weak self] in self?.refresh() }
        refresh(animated: false)
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        guard window == nil else { return refresh(animated: false) }
        timer?.invalidate()
        timer = nil
        // Leaving the screen discards a recording nobody can stop; words
        // already being transcribed still land in this draft.
        if let controller, controller.isOwned(by: self), controller.state != .transcribing {
            controller.cancel()
        }
    }

    // MARK: State

    private func refresh(animated: Bool = true) {
        var look = Look.hidden
        if let controller {
            let mine = controller.isOwned(by: self)
            switch controller.state {
            case let .recording(since) where mine:
                look = .recording(seconds: max(0, Int(Date().timeIntervalSince(since))))
            case .transcribing where mine:
                look = .transcribing
            default:
                // Another composer dictating dims this one's microphone.
                if controller.offered || mine { look = .mic(enabled: !controller.busy) }
            }
        }
        apply(look, animated: animated)
        let recording = look.sameKind(as: .recording(seconds: 0))
        if recording, timer == nil, window != nil {
            timer = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated { self?.refresh(animated: false) }
            }
        } else if !recording {
            timer?.invalidate()
            timer = nil
        }
    }

    private func apply(_ look: Look, animated: Bool) {
        guard look != shown else { return }
        let reshaped = !look.sameKind(as: shown)
        shown = look
        var config = UIButton.Configuration.filled()
        config.cornerStyle = .capsule
        config.contentInsets = .zero
        config.baseForegroundColor = Palette.text
        config.baseBackgroundColor = Palette.controlFill
        switch look {
        case .hidden:
            break
        case let .mic(enabled):
            config.image = UIImage(systemName: "mic", withConfiguration: UIImage.SymbolConfiguration(pointSize: 15, weight: .semibold))
            micButton.isEnabled = enabled
            micButton.accessibilityLabel = "Dictate with Codex"
            micButton.accessibilityValue = nil
            micButton.accessibilityHint = "Records until you tap again. ChatGPT types what you said into your message."
        case let .recording(seconds):
            config.image = UIImage(systemName: "stop.fill", withConfiguration: UIImage.SymbolConfiguration(pointSize: 10, weight: .bold))
            config.imagePadding = 6
            config.title = String(format: "%d:%02d", seconds / 60, seconds % 60)
            config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { attrs in
                var a = attrs
                a.font = Fonts.ui(.monoMedium, 13)
                return a
            }
            config.baseForegroundColor = .white
            config.baseBackgroundColor = Palette.danger
            config.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: 11, bottom: 0, trailing: 12)
            micButton.isEnabled = true
            micButton.accessibilityLabel = "Stop dictation"
            micButton.accessibilityValue = seconds == 1 ? "1 second" : "\(seconds) seconds"
            micButton.accessibilityHint = "ChatGPT transcribes what you said."
        case .transcribing:
            config.showsActivityIndicator = true
            config.baseForegroundColor = Palette.secondary
            micButton.isEnabled = true
            micButton.accessibilityLabel = "Transcribing dictation"
            micButton.accessibilityValue = nil
            micButton.accessibilityHint = nil
        }
        let visible = look != .hidden
        let cancellable = look.sameKind(as: .recording(seconds: 0)) || look == .transcribing
        let change = {
            self.micButton.configuration = config
            self.micButton.isHidden = !visible
            self.cancelButton.isHidden = !cancellable
            // Off before on: one leading edge at a time.
            NSLayoutConstraint.deactivate([self.collapse, self.leadMic, self.leadCancel])
            if !visible {
                self.collapse.isActive = true
            } else if cancellable {
                self.leadCancel.isActive = true
            } else {
                self.leadMic.isActive = true
            }
            self.gap.constant = visible ? -6 : 0
            self.roundWidth.isActive = !look.sameKind(as: .recording(seconds: 0))
            self.composer?.superview?.layoutIfNeeded()
        }
        guard animated, reshaped, window != nil, !UIAccessibility.isReduceMotionEnabled else { return change() }
        UIView.animate(withDuration: 0.32, delay: 0, usingSpringWithDamping: 0.8, initialSpringVelocity: 0, options: [.allowUserInteraction, .beginFromCurrentState], animations: change)
        if visible { micButton.imageView?.addSymbolEffect(.bounce.byLayer, options: .nonRepeating) }
    }

    // MARK: Actions

    private func micTapped() {
        guard let controller else { return }
        switch controller.state {
        case .idle:
            guard !controller.busy else { return }
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
            controller.start(owner: self) { [weak self] outcome in self?.finished(outcome) }
        case .recording:
            guard controller.isOwned(by: self) else { return }
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
            controller.stop()
        case .transcribing:
            break
        }
    }

    private func cancelTapped() {
        guard let controller, controller.isOwned(by: self) else { return }
        UIImpactFeedbackGenerator(style: .rigid).impactOccurred()
        controller.cancel()
    }

    private func finished(_ outcome: CodexDictationController.Outcome) {
        switch outcome {
        case let .text(words):
            composer?.insertDictation(words)
            UISelectionFeedbackGenerator().selectionChanged()
        case .noSpeech:
            show("No speech detected")
        case .microphoneDenied:
            UINotificationFeedbackGenerator().notificationOccurred(.error)
            show("Zeron Nova needs microphone access for Codex dictation.", action: "Settings") {
                if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) }
            }
        case let .failed(message):
            UINotificationFeedbackGenerator().notificationOccurred(.error)
            show(message)
        }
    }

    /// Notes float just above the composer: a Toast would sit under the
    /// keyboard while typing.
    private func show(_ text: String, action: String? = nil, handler: (() -> Void)? = nil) {
        notice?.dismiss(animated: false)
        guard let composer, composer.window != nil, let root = composer.findViewController()?.view else { return }
        let notice = DictationNotice(text: text, action: action, handler: handler)
        root.addSubview(notice)
        NSLayoutConstraint.activate([
            notice.centerXAnchor.constraint(equalTo: composer.centerXAnchor),
            notice.bottomAnchor.constraint(equalTo: composer.topAnchor, constant: -8),
            notice.widthAnchor.constraint(lessThanOrEqualTo: composer.widthAnchor),
        ])
        self.notice = notice
        notice.present()
    }
}

extension ComposerBar {
    /// Dictated words go where typing would: over the selection while
    /// editing, otherwise after the draft, spaced from their neighbors.
    /// Never sends.
    func insertDictation(_ words: String) {
        let draft = text as NSString
        var range = NSRange(location: draft.length, length: 0)
        if textView.isFirstResponder {
            let selected = textView.selectedRange
            if selected.location != NSNotFound, NSMaxRange(selected) <= draft.length { range = selected }
        }
        func scalar(at index: Int) -> Unicode.Scalar? {
            guard index >= 0, index < draft.length else { return nil }
            return Unicode.Scalar(draft.character(at: index))
        }
        let before = scalar(at: range.location - 1).map { !CharacterSet.whitespacesAndNewlines.contains($0) } ?? false
        let after = scalar(at: NSMaxRange(range)).map { CharacterSet.alphanumerics.contains($0) } ?? false
        let inserted = (before ? " " : "") + words + (after ? " " : "")
        text = draft.replacingCharacters(in: range, with: inserted)
        textView.selectedRange = NSRange(location: range.location + (inserted as NSString).length, length: 0)
    }
}

/// A short glass note above the composer, with an optional action.
private final class DictationNotice: UIView {
    private let glass = Glass.surface(radius: 18)
    private var dismissWork: DispatchWorkItem?

    init(text: String, action: String?, handler: (() -> Void)?) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        glass.translatesAutoresizingMaskIntoConstraints = false
        addSubview(glass)
        let label = UILabel()
        label.text = text
        label.font = Fonts.ui(.sansMedium, 14)
        label.textColor = Palette.text
        label.numberOfLines = 3
        let stack = UIStackView(arrangedSubviews: [label])
        stack.spacing = 14
        stack.alignment = .center
        if let action {
            var c = UIButton.Configuration.plain()
            c.title = action
            c.baseForegroundColor = Palette.accent
            c.contentInsets = .zero
            c.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { a in
                var a = a
                a.font = Fonts.ui(.sansSemibold, 14)
                return a
            }
            let b = UIButton(configuration: c, primaryAction: UIAction { [weak self] _ in
                handler?()
                self?.dismiss(animated: true)
            })
            b.setContentCompressionResistancePriority(.required, for: .horizontal)
            b.accessibilityIdentifier = "composer-dictation-notice-action"
            stack.addArrangedSubview(b)
        }
        stack.translatesAutoresizingMaskIntoConstraints = false
        glass.contentView.addSubview(stack)
        NSLayoutConstraint.activate([
            glass.topAnchor.constraint(equalTo: topAnchor),
            glass.bottomAnchor.constraint(equalTo: bottomAnchor),
            glass.leadingAnchor.constraint(equalTo: leadingAnchor),
            glass.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: glass.contentView.topAnchor, constant: 10),
            stack.bottomAnchor.constraint(equalTo: glass.contentView.bottomAnchor, constant: -10),
            stack.leadingAnchor.constraint(equalTo: glass.contentView.leadingAnchor, constant: 16),
            stack.trailingAnchor.constraint(equalTo: glass.contentView.trailingAnchor, constant: -16),
        ])
        accessibilityIdentifier = "composer-dictation-notice"
    }

    required init?(coder: NSCoder) { fatalError() }

    func present() {
        Glass.setVisible(glass, false, animated: false)
        transform = CGAffineTransform(translationX: 0, y: 8)
        UIView.animate(withDuration: 0.4, delay: 0, usingSpringWithDamping: 0.85, initialSpringVelocity: 0) {
            self.transform = .identity
        }
        Glass.setVisible(glass, true)
        UIAccessibility.post(notification: .announcement, argument: (glass.contentView.subviews.first?.subviews.first as? UILabel)?.text)
        let work = DispatchWorkItem { [weak self] in self?.dismiss(animated: true) }
        dismissWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 4, execute: work)
    }

    func dismiss(animated: Bool) {
        dismissWork?.cancel()
        guard animated else { return removeFromSuperview() }
        Glass.setVisible(glass, false)
        UIView.animate(withDuration: 0.25, animations: {
            self.transform = CGAffineTransform(translationX: 0, y: 8)
        }, completion: { _ in self.removeFromSuperview() })
    }
}
