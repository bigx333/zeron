import UIKit

/// A model's effort ladder, as the effort control shows it.
struct EffortChoice: Equatable {
    /// Wire values, low to high.
    let levels: [String]
    /// The level in effect.
    let current: String?
    /// The model's own default (the reset target).
    let defaultLevel: String?
}

/// The effort slider in a small popover anchored to the composer's effort
/// chip (a menu can't host it). A level applies as soon as the thumb settles;
/// the popover stays up for another try and closes with Done or a tap
/// outside.
final class EffortSheetController: UIViewController, UIPopoverPresentationControllerDelegate {
    private let choice: EffortChoice
    private let onPick: (String) -> Void
    private let slider = EffortSlider()
    private let stack = UIStackView()
    private static let padding: CGFloat = 20

    init(choice: EffortChoice, onPick: @escaping (String) -> Void) {
        self.choice = choice
        self.onPick = onPick
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    /// Show the slider for `choice` by `anchor` (the chip, which must be in
    /// `host`'s view). `onPick` gets each applied level.
    @MainActor
    static func present(from host: UIViewController, anchor: UIView, choice: EffortChoice, onPick: @escaping (String) -> Void) {
        guard !choice.levels.isEmpty, host.presentedViewController == nil else { return }
        let vc = EffortSheetController(choice: choice, onPick: onPick)
        vc.modalPresentationStyle = .popover
        let width = min(360, max(260, host.view.bounds.width - 32))
        vc.loadViewIfNeeded()
        vc.preferredContentSize = vc.fittingSize(width: width)
        if let popover = vc.popoverPresentationController {
            // The chip is rebuilt when its title changes (after a pick): hold
            // on to where it was, in the host's view.
            popover.sourceView = host.view
            popover.sourceRect = anchor.convert(anchor.bounds, to: host.view)
            popover.permittedArrowDirections = [.down, .up]
            // A popover on iPhone too: it sits by the chip, above the keyboard,
            // and leaves the composer focused.
            popover.delegate = vc
        }
        host.present(vc, animated: true)
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        // The popover's own (glass) background shows through.
        view.backgroundColor = .clear
        let title = UILabel()
        title.text = "Reasoning Effort"
        title.font = Fonts.ui(.sansMedium, TypeScale.size(13))
        title.textColor = Palette.secondary
        title.accessibilityTraits = .header

        var doneConfig = UIButton.Configuration.plain()
        doneConfig.title = "Done"
        doneConfig.baseForegroundColor = Palette.accent
        doneConfig.contentInsets = NSDirectionalEdgeInsets(top: 4, leading: 8, bottom: 4, trailing: 0)
        doneConfig.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { attrs in
            var a = attrs
            a.font = Fonts.ui(.sansSemibold, TypeScale.size(15))
            return a
        }
        let done = UIButton(configuration: doneConfig)
        done.accessibilityIdentifier = "effort-done"
        done.setContentHuggingPriority(.required, for: .horizontal)
        done.addAction(UIAction { [weak self] _ in self?.dismiss(animated: true) }, for: .primaryActionTriggered)
        let titleRow = UIStackView(arrangedSubviews: [title, done])
        titleRow.axis = .horizontal
        titleRow.alignment = .center

        slider.configure(levels: choice.levels, level: choice.current, defaultLevel: choice.defaultLevel)
        slider.addAction(UIAction { [weak self] _ in
            guard let self else { return }
            self.onPick(self.slider.level)
        }, for: .valueChanged)

        stack.axis = .vertical
        stack.spacing = 4
        stack.translatesAutoresizingMaskIntoConstraints = false
        stack.addArrangedSubview(titleRow)
        stack.addArrangedSubview(slider)
        view.addSubview(stack)
        let guide = view.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: guide.topAnchor, constant: Self.padding - 4),
            stack.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: Self.padding),
            stack.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -Self.padding),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: guide.bottomAnchor, constant: -Self.padding),
        ])
    }

    /// The content's height at `width` (the popover adds its arrow outside).
    private func fittingSize(width: CGFloat) -> CGSize {
        let inner = stack.systemLayoutSizeFitting(
            CGSize(width: width - 2 * Self.padding, height: UIView.layoutFittingCompressedSize.height),
            withHorizontalFittingPriority: .required,
            verticalFittingPriority: .fittingSizeLevel
        )
        return CGSize(width: width, height: ceil(inner.height) + 2 * Self.padding - 4)
    }

    // MARK: UIPopoverPresentationControllerDelegate

    func adaptivePresentationStyle(for controller: UIPresentationController, traitCollection: UITraitCollection) -> UIModalPresentationStyle {
        .none
    }
}
