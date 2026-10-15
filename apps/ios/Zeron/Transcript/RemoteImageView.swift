import UIKit

/// A transcript thumbnail loaded from the chat's host: a soft sweep while it
/// loads so it never reads as broken, a fade-in when it lands, and a photo
/// glyph (tap to retry) when it can't be fetched.
final class RemoteImageView: UIImageView {
    /// Fetches the image again after a failure; set by the loader.
    var retry: (() -> Void)?
    /// Each request owns the view, even when a reused view returns to the same path.
    var loadID = UUID()
    private(set) var failed = false
    private var loading = false
    private let sweep = CAGradientLayer()
    private let glyph = UIImageView(image: UIImage(systemName: "photo"))

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .secondarySystemFill
        sweep.startPoint = CGPoint(x: 0, y: 0.3)
        sweep.endPoint = CGPoint(x: 1, y: 0.7)
        sweep.isHidden = true
        layer.addSublayer(sweep)
        glyph.tintColor = .tertiaryLabel
        glyph.contentMode = .scaleAspectFit
        glyph.isHidden = true
        addSubview(glyph)
        registerForTraitChanges([UITraitUserInterfaceStyle.self, UITraitAccessibilityContrast.self]) {
            (view: RemoteImageView, _: UITraitCollection) in view.setNeedsLayout()
        }
        NotificationCenter.default.addObserver(self, selector: #selector(reduceMotionChanged),
            name: UIAccessibility.reduceMotionStatusDidChangeNotification, object: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func layoutSubviews() {
        super.layoutSubviews()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        sweep.frame = bounds
        // Resolved here so it follows light/dark changes.
        let highlight = UIColor.label.withAlphaComponent(0.08).resolvedColor(with: traitCollection)
        sweep.colors = [UIColor.clear.cgColor, highlight.cgColor, UIColor.clear.cgColor]
        CATransaction.commit()
        let side = min(28, min(bounds.width, bounds.height) * 0.4)
        glyph.frame = CGRect(x: bounds.midX - side / 2, y: bounds.midY - side / 2, width: side, height: side)
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        // Restart on attachment even if an old, detached animation is retained.
        sweep.removeAnimation(forKey: "sweep")
        if loading { animateSweep() }
    }

    func showLoading() {
        layer.removeAllAnimations()
        accessibilityLabel = nil
        failed = false
        glyph.isHidden = true
        image = nil
        loading = true
        sweep.isHidden = false
        animateSweep()
    }

    func show(_ image: UIImage, animated: Bool) {
        layer.removeAllAnimations()
        stopSweep()
        retry = nil
        failed = false
        glyph.isHidden = true
        guard animated, !UIAccessibility.isReduceMotionEnabled else {
            self.image = image
            return
        }
        UIView.transition(with: self, duration: 0.22, options: .transitionCrossDissolve) { self.image = image }
    }

    func showFailed() {
        layer.removeAllAnimations()
        stopSweep()
        image = nil
        failed = true
        glyph.isHidden = false
        accessibilityLabel = "Image unavailable, tap to retry"
    }

    private func animateSweep() {
        guard loading, window != nil, !UIAccessibility.isReduceMotionEnabled,
              sweep.animation(forKey: "sweep") == nil else { return }
        let sweepAcross = CABasicAnimation(keyPath: "locations")
        sweepAcross.fromValue = [-0.4, -0.2, 0.0]
        sweepAcross.toValue = [1.0, 1.2, 1.4]
        sweepAcross.duration = 1.3
        sweepAcross.repeatCount = .infinity
        sweep.add(sweepAcross, forKey: "sweep")
    }

    @objc private func reduceMotionChanged() {
        sweep.removeAnimation(forKey: "sweep")
        if loading { animateSweep() }
    }

    private func stopSweep() {
        loading = false
        sweep.removeAnimation(forKey: "sweep")
        sweep.isHidden = true
        accessibilityLabel = nil
    }
}
