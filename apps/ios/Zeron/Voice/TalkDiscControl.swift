import UIKit

/// The primary voice action: a labelled disc that rises out of the call bar.
/// The larger hit area includes its dotted microphone meter.
final class TalkDiscControl: UIControl {
    private let artwork = UIView()
    private let moat = UIView()
    private let plate = UIView()
    private let glyph = UIImageView(image: UIImage(systemName: "mic.fill", withConfiguration: UIImage.SymbolConfiguration(pointSize: 28, weight: .medium)))
    private let label = UILabel()
    private let ring = CAShapeLayer()
    private var held = false
    private var microphone: Float = 0

    override init(frame: CGRect) {
        super.init(frame: frame)
        isAccessibilityElement = true
        accessibilityLabel = "Hold to talk"
        accessibilityHint = "Double-tap and hold to talk. Release to mute."
        artwork.isUserInteractionEnabled = false
        addSubview(artwork)
        artwork.layer.addSublayer(ring)
        for v in [moat, plate, glyph, label] { artwork.addSubview(v) }
        label.textAlignment = .center
        label.font = Fonts.ui(.sansSemibold, 11.5)
        label.adjustsFontSizeToFitWidth = true
        label.minimumScaleFactor = 0.8
        glyph.contentMode = .scaleAspectFit
        registerForTraitChanges([UITraitUserInterfaceStyle.self, UITraitPreferredContentSizeCategory.self]) { (self: TalkDiscControl, _) in
            self.paint(animated: false)
        }
        paint(animated: false)
    }

    required init?(coder: NSCoder) { fatalError() }

    func update(active: Bool, muted: Bool, microphone: Float) {
        let held = active && !muted
        let changed = held != self.held
        if !active && isEnabled { sendActions(for: .touchCancel) }
        isEnabled = active
        self.held = held
        self.microphone = microphone.isFinite ? max(0, min(1, microphone)) : 0
        paint(animated: changed)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        artwork.bounds = bounds
        artwork.center = CGPoint(x: bounds.midX, y: bounds.midY)
        moat.bounds = CGRect(x: 0, y: 0, width: 108, height: 108)
        moat.center = CGPoint(x: bounds.midX, y: bounds.midY)
        moat.layer.cornerRadius = 54
        plate.bounds = CGRect(x: 0, y: 0, width: 96, height: 96)
        plate.center = moat.center
        plate.layer.cornerRadius = 48
        glyph.frame = CGRect(x: bounds.midX - 14, y: bounds.midY - 26, width: 28, height: 28)
        label.frame = CGRect(x: bounds.midX - 44, y: bounds.midY + 7, width: 88, height: 20)
        ring.frame = bounds
        paint(animated: false)
    }

    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        let dx = point.x - bounds.midX, dy = point.y - bounds.midY
        return isEnabled && dx * dx + dy * dy <= 66 * 66
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window == nil { sendActions(for: .touchCancel) }
        paint(animated: false)
    }

    private func paint(animated: Bool) {
        let reducedMotion = UIAccessibility.isReduceMotionEnabled
        let text = Palette.text.resolvedColor(with: traitCollection)
        let success = Palette.success.resolvedColor(with: traitCollection)
        let background = Palette.background.resolvedColor(with: traitCollection)
        moat.backgroundColor = background
        plate.backgroundColor = held ? text : Palette.dynamic(0xE7E7EB, 0x222227)
        plate.layer.borderWidth = 1
        plate.layer.borderColor = (held ? success : text.withAlphaComponent(0.14)).cgColor
        plate.layer.shadowPath = UIBezierPath(ovalIn: plate.bounds).cgPath
        plate.layer.shadowColor = success.cgColor
        plate.layer.shadowOffset = .zero
        plate.layer.shadowRadius = 20
        plate.layer.shadowOpacity = held ? 0.25 : 0
        glyph.tintColor = held ? background : text
        label.textColor = glyph.tintColor
        label.text = !isEnabled ? "Connecting" : held ? "Talking" : "Hold to talk"
        let fontScale = min(1.3, UIFontMetrics(forTextStyle: .caption1).scaledValue(for: 1, compatibleWith: traitCollection))
        label.font = Fonts.ui(.sansSemibold, 11.5 * fontScale)
        alpha = isEnabled ? 1 : 0.42
        accessibilityTraits = isEnabled ? [.button, .startsMediaSession] : [.button, .notEnabled]
        accessibilityValue = !isEnabled ? "Unavailable while connecting" : held ? "Transmitting" : "Microphone off"

        let path = UIBezierPath()
        if isEnabled {
            for i in 0..<44 {
                let angle = CGFloat(i) * 2 * .pi / 44
                let level = held && !reducedMotion ? CGFloat(microphone) : 0
                let radius = 58 + level * 7 * (0.6 + 0.4 * sin(angle * 3))
                let center = CGPoint(x: bounds.midX + cos(angle) * radius, y: bounds.midY + sin(angle) * radius)
                path.append(UIBezierPath(ovalIn: CGRect(x: center.x - 1.6, y: center.y - 1.6, width: 3.2, height: 3.2)))
            }
        }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        ring.path = path.cgPath
        ring.fillColor = (held ? success : text).cgColor
        ring.opacity = held ? 1 : 0.45
        CATransaction.commit()
        if isEnabled && !held && !reducedMotion && window != nil {
            if ring.animation(forKey: "breath") == nil {
                let breath = CABasicAnimation(keyPath: "opacity")
                breath.fromValue = 0.3; breath.toValue = 0.55
                breath.duration = 1.2; breath.autoreverses = true; breath.repeatCount = .infinity
                ring.add(breath, forKey: "breath")
            }
        } else { ring.removeAnimation(forKey: "breath") }
        let transform = held && !reducedMotion ? CGAffineTransform(scaleX: 1.06, y: 1.06) : .identity
        if animated && !reducedMotion {
            UIView.animate(withDuration: 0.22, delay: 0, usingSpringWithDamping: 0.6, initialSpringVelocity: 0, options: [.beginFromCurrentState, .allowUserInteraction]) {
                self.artwork.transform = transform
            }
        } else { artwork.transform = transform }
    }
}
