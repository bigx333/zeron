import UIKit

/// The effort slider's pure parts: tiers, the heat palette, track geometry,
/// motion and sparks (kept apart from the view so they can be tested).
/// Mirrors Android's `EffortSlider.kt`.
enum EffortScale {
    /// The top tiers (Codex's Ultra, Claude's Ultracode and Ultrathink).
    static func isPeak(_ level: String) -> Bool {
        level == "ultra" || level == "ultracode" || level == "ultrathink"
    }

    // MARK: Heat

    struct RGB: Equatable {
        var r: CGFloat
        var g: CGFloat
        var b: CGFloat

        init(r: CGFloat, g: CGFloat, b: CGFloat) {
            self.r = r
            self.g = g
            self.b = b
        }

        init(hex: UInt32) {
            r = CGFloat((hex >> 16) & 0xFF) / 255
            g = CGFloat((hex >> 8) & 0xFF) / 255
            b = CGFloat(hex & 0xFF) / 255
        }

        var color: UIColor { UIColor(red: r, green: g, blue: b, alpha: 1) }
    }

    /// Effort as heat: calm sky at the low end, through blue, indigo and
    /// violet, to fuchsia at the top.
    static let heatStops: [(at: CGFloat, color: RGB)] = [
        (0, RGB(hex: 0x38BDF8)),
        (0.35, RGB(hex: 0x3B82F6)),
        (0.6, RGB(hex: 0x6366F1)),
        (0.82, RGB(hex: 0x8B5CF6)),
        (1, RGB(hex: 0xD946EF)),
    ]

    /// The heat at `at` (0…1), linear between the stops.
    static func heat(_ at: CGFloat) -> RGB {
        let f = min(1, max(0, at))
        let i = max(1, heatStops.firstIndex { $0.at >= f } ?? heatStops.count - 1)
        let (a, ca) = heatStops[i - 1]
        let (b, cb) = heatStops[i]
        let t = b > a ? (f - a) / (b - a) : 1
        return RGB(r: ca.r + (cb.r - ca.r) * t, g: ca.g + (cb.g - ca.g) * t, b: ca.b + (cb.b - ca.b) * t)
    }

    /// The label and icon color: never lighter than blue, so it reads on white.
    static func accent(fraction: CGFloat) -> RGB { heat(0.35 + 0.65 * fraction) }

    /// The fill's gradient: 7 stops of the band of heat ending at the level
    /// (calm sky at Low, indigo to fuchsia end to end at the top).
    static func fillBand(fraction: CGFloat) -> [RGB] {
        let from = max(0, fraction - 0.5)
        return (0...6).map { heat(from + (fraction - from) * CGFloat($0) / 6) }
    }

    /// 0 below `from`, 1 above `to`, smoothstepped between.
    static func ramp(_ from: CGFloat, _ to: CGFloat, _ x: CGFloat) -> CGFloat {
        let t = min(1, max(0, (x - from) / (to - from)))
        return t * t * (3 - 2 * t)
    }

    /// The glow around the filled track: from Extra High up, breathing,
    /// strongest in the top tiers.
    static func glow(fraction: CGFloat, breath: Double, top: CGFloat) -> CGFloat {
        ramp(0.55, 1, fraction) * (0.55 + 0.45 * breathWave(breath)) * (0.7 + 0.3 * top)
    }

    static func breathWave(_ breath: Double) -> CGFloat {
        CGFloat(0.5 + 0.5 * sin(2 * Double.pi * breath))
    }

    // MARK: Geometry

    /// Where the thumb rests for `step` of `count` levels (0…1).
    static func fraction(ofStep step: Int, count: Int) -> CGFloat {
        count > 1 ? CGFloat(min(max(step, 0), count - 1)) / CGFloat(count - 1) : 0
    }

    /// The track position (0…1) under `x`. The thumb's center runs between
    /// the centers of the two end caps.
    static func fraction(atX x: CGFloat, width: CGFloat, height: CGFloat) -> CGFloat {
        let r = height / 2
        let run = width - 2 * r
        guard run > 0 else { return 0 }
        return min(1, max(0, (x - r) / run))
    }

    /// The nearest step to a track position.
    static func step(atFraction f: CGFloat, count: Int) -> Int {
        guard count > 1 else { return 0 }
        return min(count - 1, max(0, Int((min(1, max(0, f)) * CGFloat(count - 1)).rounded())))
    }

    static func step(atX x: CGFloat, width: CGFloat, height: CGFloat, count: Int) -> Int {
        step(atFraction: fraction(atX: x, width: width, height: height), count: count)
    }

    // MARK: Motion

    /// The slider's motion, advanced every frame at a pace set by the effort,
    /// so a change of level speeds it up or calms it down without a jump.
    struct Motion: Equatable {
        /// Drift of the sparks, in fill widths.
        var drift: Double = 0
        /// The sheen's sweep across the fill, 0..<1.
        var sweep: Double = 0
        /// A slow breath for the glow and halo, in cycles.
        var breath: Double = 0

        mutating func advance(_ seconds: Double, intensity: CGFloat) {
            let i = Double(min(1, max(0, intensity)))
            drift += seconds * (0.03 + 0.17 * i)
            sweep = (sweep + seconds / (3.4 - 2.2 * i)).truncatingRemainder(dividingBy: 1)
            breath += seconds * (0.4 + 0.6 * i)
        }
    }

    // MARK: Sparks

    /// One drifting point of light in the filled track. Positions are
    /// fractions of the fill; speed scales the drift.
    struct Spark: Equatable {
        let x: CGFloat
        let y: CGFloat
        let radius: CGFloat
        let speed: CGFloat
        let phase: CGFloat
        let twinkle: CGFloat
    }

    static let sparkCount = 44

    /// The same sparks on every launch (seeded).
    static let sparks: [Spark] = {
        var random = SeededRandom(seed: 7)
        return (0..<sparkCount).map { _ in
            let x = random.unit()
            let y = 0.18 + 0.64 * random.unit()
            let radius = 0.8 + 1.4 * random.unit()
            let speed = 0.5 + random.unit()
            let phase = random.unit()
            let twinkle = 1 + 2 * random.unit()
            return Spark(x: x, y: y, radius: radius, speed: speed, phase: phase, twinkle: twinkle)
        }
    }()

    /// Sparks shown at `fraction`: 8 at the lowest effort, all 44 at the top.
    static func visibleSparks(fraction: CGFloat) -> Int {
        let f = min(1, max(0, fraction))
        return min(sparkCount, Int(8 + CGFloat(sparkCount - 8) * f))
    }

    /// A spark's x (0…1 of the fill width) once the field has drifted.
    static func sparkX(_ s: Spark, drift: Double) -> CGFloat {
        let v = (Double(s.x) + drift * Double(s.speed)).truncatingRemainder(dividingBy: 1)
        return CGFloat(v < 0 ? v + 1 : v)
    }

    /// 0…1 brightness pulse.
    static func twinkle(_ s: Spark, breath: Double) -> CGFloat {
        CGFloat(abs(sin(2 * Double.pi * (breath * Double(s.twinkle) + Double(s.phase)))))
    }

    static func sparkAlpha(_ s: Spark, breath: Double, fraction: CGFloat) -> CGFloat {
        (0.3 + 0.6 * twinkle(s, breath: breath)) * (0.7 + 0.3 * fraction)
    }

    /// Streak length behind a speed-1 spark, in points (none below 0.6).
    static func streak(fraction: CGFloat) -> CGFloat { ramp(0.6, 1, fraction) * 14 }

    /// The top tiers' halo: how far out it has pulsed (0..<1).
    static func haloWave(breath: Double) -> CGFloat {
        CGFloat((breath * 1.5).truncatingRemainder(dividingBy: 1))
    }

    /// SplitMix64: a small deterministic generator for the spark field.
    struct SeededRandom: RandomNumberGenerator {
        private var state: UInt64
        init(seed: UInt64) { state = seed }

        mutating func next() -> UInt64 {
            state &+= 0x9E37_79B9_7F4A_7C15
            var z = state
            z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
            z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
            return z ^ (z >> 31)
        }

        /// Uniform in 0..<1.
        mutating func unit() -> CGFloat {
            CGFloat(Double(next() >> 11) * 0x1.0p-53)
        }
    }
}

/// The model's effort levels on a stepped track, low to high, named above it
/// in the level's color. Effort reads as heat building up: the fill warms from
/// sky to fuchsia, a sheen sweeps it, sparks thicken and quicken (streaking at
/// the high end), a glow breathes around the track from Extra High up, and the
/// top tiers pulse a halo from the thumb. The thumb springs to each step with
/// a tick (a firmer one into the top tiers); the choice applies when it
/// settles (tap or release): `.valueChanged` fires then, with `level`
/// updated. Reset returns to the model's default.
///
/// Animates on a display link while on screen; off-window, hidden or
/// backgrounded it stops. Reduce Motion draws still frames (no link): the
/// thumb jumps to its step.
final class EffortSlider: UIControl {
    /// The levels, low to high (wire values).
    private(set) var levels: [String] = []
    /// The applied level.
    private(set) var level = ""
    private(set) var defaultLevel: String?

    private let header = UIStackView()
    private let icon = UIImageView()
    private let nameLabel = UILabel()
    private let resetButton = UIButton(type: .system)
    private let track = EffortTrackView()
    /// The breathing glow behind the filled part (its shadow is the glow).
    private let glowLayer = CAShapeLayer()
    /// The top tiers' ring pulsing out from the thumb.
    private let haloLayer = CAShapeLayer()
    private let stepTick = UISelectionFeedbackGenerator()
    private let peakThud = UIImpactFeedbackGenerator(style: .medium)
    private let horizontalOnly = HorizontalPanDelegate()

    /// The step under the thumb (ahead of `level` while dragging).
    private var step = 0
    /// The finger's track position while dragging.
    private var dragging: CGFloat?
    /// The thumb's sprung position and velocity (track fractions).
    private var shown: CGFloat = 0
    private var velocity: CGFloat = 0
    /// 0…1 while in a top tier, eased over half a second.
    private var top: CGFloat = 0
    private var motion = EffortScale.Motion()
    private var link: CADisplayLink?
    private var lastTick: CFTimeInterval?
    private var observers: [NSObjectProtocol] = []
    private var shownName: String?
    private var renderedAccent: EffortScale.RGB?

    private static let trackHeight: CGFloat = 44
    private static let topDuration: CGFloat = 0.5
    /// Spring: stiffness 400, damping ratio 0.75 (Compose's MediumLow).
    private static let stiffness: CGFloat = 400
    private static let damping: CGFloat = 2 * 0.75 * 20

    override init(frame: CGRect) {
        super.init(frame: frame)
        build()
    }

    required init?(coder: NSCoder) { fatalError() }

    deinit {
        link?.invalidate()
        observers.forEach(NotificationCenter.default.removeObserver)
    }

    private func build() {
        icon.image = UIImage(systemName: "gauge.with.dots.needle.67percent", withConfiguration: UIImage.SymbolConfiguration(pointSize: 17, weight: .semibold))
        icon.contentMode = .center
        icon.setContentHuggingPriority(.required, for: .horizontal)
        icon.isAccessibilityElement = false

        nameLabel.font = Fonts.ui(.sansSemibold, TypeScale.size(17))
        nameLabel.adjustsFontSizeToFitWidth = true
        nameLabel.minimumScaleFactor = 0.8
        // The track carries the level for VoiceOver.
        nameLabel.isAccessibilityElement = false
        nameLabel.setContentHuggingPriority(.defaultLow, for: .horizontal)

        resetButton.setImage(UIImage(systemName: "arrow.counterclockwise", withConfiguration: UIImage.SymbolConfiguration(pointSize: 16, weight: .semibold)), for: .normal)
        resetButton.tintColor = Palette.secondary
        resetButton.accessibilityLabel = "Reset to the model's default"
        resetButton.accessibilityIdentifier = "effort-reset"
        resetButton.setContentHuggingPriority(.required, for: .horizontal)
        resetButton.addAction(UIAction { [weak self] _ in self?.reset() }, for: .primaryActionTriggered)
        resetButton.isHidden = true
        resetButton.alpha = 0

        header.axis = .horizontal
        header.alignment = .center
        header.spacing = 10
        for v in [icon, nameLabel, resetButton] as [UIView] { header.addArrangedSubview(v) }

        track.isAccessibilityElement = true
        track.accessibilityLabel = "Reasoning effort"
        track.accessibilityTraits = .adjustable
        track.accessibilityIdentifier = "effort-track"
        track.onAdjust = { [weak self] delta in
            guard let self else { return }
            self.pick(self.step + delta)
            self.commit()
        }
        let pan = UIPanGestureRecognizer(target: self, action: #selector(panned(_:)))
        pan.delegate = horizontalOnly
        track.addGestureRecognizer(pan)
        track.addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(tapped(_:))))

        let stack = UIStackView(arrangedSubviews: [header, track])
        stack.axis = .vertical
        stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        // Below required: the stack collapses the button while it is hidden.
        let resetWidth = resetButton.widthAnchor.constraint(equalToConstant: 40)
        resetWidth.priority = UILayoutPriority(999)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: topAnchor),
            resetWidth,
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            header.heightAnchor.constraint(greaterThanOrEqualToConstant: 40),
            icon.widthAnchor.constraint(equalToConstant: 22),
            resetButton.heightAnchor.constraint(equalToConstant: 40),
            track.heightAnchor.constraint(equalToConstant: Self.trackHeight),
        ])

        // The glow sits under the track (only its shadow shows past the
        // fill); the halo over it, both free to spill past the track's edge.
        glowLayer.shadowOffset = .zero
        glowLayer.shadowRadius = 8
        glowLayer.shadowOpacity = 0
        glowLayer.opacity = 0
        layer.insertSublayer(glowLayer, at: 0)
        haloLayer.fillColor = nil
        haloLayer.lineWidth = 2
        haloLayer.opacity = 0
        layer.addSublayer(haloLayer)

        accessibilityElements = [track, resetButton]
        accessibilityIdentifier = "effort-slider"

        // Resume on didBecomeActive: at willEnterForeground the app still
        // reports .background, so the display link would stay stopped.
        for name in [UIApplication.didEnterBackgroundNotification, UIApplication.didBecomeActiveNotification, UIAccessibility.reduceMotionStatusDidChangeNotification] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.frameChanged() }
            })
        }
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) { (self: EffortSlider, _) in self.track.setNeedsDisplay() }
    }

    // MARK: State

    /// Show `levels` with `level` chosen (falling back to the default, then
    /// the middle step). Resets any drag and settles without animating.
    func configure(levels: [String], level: String?, defaultLevel: String?) {
        self.levels = levels
        self.defaultLevel = defaultLevel
        let index = level.flatMap { levels.firstIndex(of: $0) } ?? defaultLevel.flatMap { levels.firstIndex(of: $0) } ?? levels.count / 2
        step = levels.isEmpty ? 0 : index
        self.level = levels.indices.contains(step) ? levels[step] : ""
        dragging = nil
        shown = target
        velocity = 0
        top = topTarget
        stepChanged(animated: false)
    }

    /// Move to `level` (a change from elsewhere: another device, a model
    /// switch). Ignored mid-drag.
    func setLevel(_ level: String, animated: Bool) {
        guard dragging == nil, let index = levels.firstIndex(of: level) else { return }
        self.level = level
        guard index != step else { return }
        step = index
        if !animated {
            shown = target
            velocity = 0
            top = topTarget
        }
        stepChanged(animated: animated)
    }

    private var target: CGFloat { EffortScale.fraction(ofStep: step, count: levels.count) }
    private var stepLevel: String { levels.indices.contains(step) ? levels[step] : level }
    private var topTarget: CGFloat { EffortScale.isPeak(stepLevel) ? 1 : 0 }
    /// Where the thumb is drawn.
    private var fraction: CGFloat { dragging ?? shown }

    private func pick(_ to: Int) {
        guard !levels.isEmpty else { return }
        let s = min(max(to, 0), levels.count - 1)
        guard s != step else { return }
        let rising = !EffortScale.isPeak(stepLevel) && EffortScale.isPeak(levels[s])
        step = s
        if rising { peakThud.impactOccurred() } else { stepTick.selectionChanged() }
        stepChanged(animated: true)
    }

    /// Apply the step under the thumb.
    private func commit() {
        guard levels.indices.contains(step), levels[step] != level else { return }
        level = levels[step]
        sendActions(for: .valueChanged)
    }

    private func reset() {
        guard let d = defaultLevel, let i = levels.firstIndex(of: d) else { return }
        pick(i)
        commit()
    }

    private func stepChanged(animated: Bool) {
        let name = stepLevel.isEmpty ? "" : reasoningLabel(level: stepLevel)
        if name != shownName {
            if animated, shownName != nil, window != nil {
                let fade = CATransition()
                fade.type = .fade
                fade.duration = 0.18
                nameLabel.layer.add(fade, forKey: "name")
            }
            shownName = name
            nameLabel.text = name
            track.accessibilityValue = name
        }
        let showReset = defaultLevel.map { $0 != stepLevel && levels.contains($0) } ?? false
        if showReset == resetButton.isHidden {
            let change = {
                self.resetButton.isHidden = !showReset
                self.resetButton.alpha = showReset ? 1 : 0
                self.resetButton.transform = showReset ? .identity : CGAffineTransform(scaleX: 0.6, y: 0.6)
            }
            if animated, window != nil, !UIAccessibility.isReduceMotionEnabled {
                UIView.animate(withDuration: 0.3, delay: 0, usingSpringWithDamping: 0.8, initialSpringVelocity: 0, options: [.beginFromCurrentState, .allowUserInteraction], animations: change)
            } else {
                change()
            }
        }
        frameChanged()
    }

    // MARK: Gestures

    @objc private func panned(_ g: UIPanGestureRecognizer) {
        let f = EffortScale.fraction(atX: g.location(in: track).x, width: track.bounds.width, height: track.bounds.height)
        switch g.state {
        case .began:
            stepTick.prepare()
            peakThud.prepare()
            dragging = f
            pick(EffortScale.step(atFraction: f, count: levels.count))
        case .changed:
            dragging = f
            pick(EffortScale.step(atFraction: f, count: levels.count))
        case .ended:
            pick(EffortScale.step(atFraction: f, count: levels.count))
            // Hand the finger's speed to the spring, so it settles with the throw.
            let run = max(1, track.bounds.width - track.bounds.height)
            shown = f
            velocity = min(8, max(-8, g.velocity(in: track).x / run))
            dragging = nil
            commit()
        default:
            // Cancelled: back to what was applied.
            shown = f
            velocity = 0
            dragging = nil
            if let i = levels.firstIndex(of: level) { step = i }
            stepChanged(animated: true)
        }
        frameChanged()
    }

    @objc private func tapped(_ g: UITapGestureRecognizer) {
        stepTick.prepare()
        pick(EffortScale.step(atX: g.location(in: track).x, width: track.bounds.width, height: track.bounds.height, count: levels.count))
        commit()
    }

    // MARK: Animation

    override var isHidden: Bool { didSet { syncLink() } }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        frameChanged()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        render()
    }

    /// Runs only on screen, and never under Reduce Motion.
    private func syncLink() {
        let run = window != nil && !isHidden && !UIAccessibility.isReduceMotionEnabled
            && UIApplication.shared.applicationState != .background
        if run {
            guard link == nil else { return }
            let link = CADisplayLink(target: EffortLinkProxy(self), selector: #selector(EffortLinkProxy.tick(_:)))
            link.preferredFrameRateRange = CAFrameRateRange(minimum: 30, maximum: 60, preferred: 60)
            link.add(to: .main, forMode: .common)
            self.link = link
        } else {
            link?.invalidate()
            link = nil
        }
        lastTick = nil
    }

    /// Something changed: the running link draws it next frame; without one
    /// (Reduce Motion, off screen) settle and draw a still frame now.
    private func frameChanged() {
        syncLink()
        guard link == nil else { return }
        if dragging == nil {
            shown = target
            velocity = 0
        }
        top = topTarget
        render()
    }

    fileprivate func tick(_ link: CADisplayLink) {
        let now = link.targetTimestamp
        let dt = lastTick.map { min(1.0 / 20, max(0, now - $0)) } ?? 0
        lastTick = now
        guard dt > 0 else { return render() }
        if let d = dragging {
            shown = d
            velocity = 0
        } else {
            let goal = target
            var remaining = CGFloat(dt)
            while remaining > 0 {
                let h = min(remaining, 1.0 / 240)
                let a = -Self.stiffness * (shown - goal) - Self.damping * velocity
                velocity += a * h
                shown += velocity * h
                remaining -= h
            }
            if abs(shown - goal) < 0.0005, abs(velocity) < 0.005 {
                shown = goal
                velocity = 0
            }
        }
        let t = topTarget
        let stepTop = CGFloat(dt) / Self.topDuration
        top = top < t ? min(t, top + stepTop) : max(t, top - stepTop)
        motion.advance(dt, intensity: fraction)
        render()
    }

    private func render() {
        let f = fraction
        let accent = EffortScale.accent(fraction: f)
        if accent != renderedAccent {
            renderedAccent = accent
            let color = accent.color
            icon.tintColor = color
            nameLabel.textColor = color
        }
        track.count = levels.count
        track.fraction = f
        track.motion = motion
        track.setNeedsDisplay()

        // Glow and halo, in this view's space.
        let frame = track.convert(track.bounds, to: self)
        let r = frame.height / 2
        let thumbX = frame.minX + r + f * max(0, frame.width - 2 * r)
        let hot = EffortScale.heat(f).color.cgColor
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let glow = EffortScale.glow(fraction: f, breath: motion.breath, top: top)
        if glow > 0.01, frame.height > 0 {
            let fill = CGRect(x: frame.minX, y: frame.minY, width: thumbX + r - frame.minX, height: frame.height)
            let path = UIBezierPath(roundedRect: fill, cornerRadius: r).cgPath
            glowLayer.path = path
            glowLayer.shadowPath = path
            glowLayer.fillColor = hot
            glowLayer.shadowColor = hot
            glowLayer.shadowOpacity = Float(min(1, glow * 0.85))
            glowLayer.opacity = 1
        } else {
            glowLayer.opacity = 0
        }
        if top > 0.01, frame.height > 0 {
            let wave = EffortScale.haloWave(breath: motion.breath)
            let radius = r - 4 + wave * 10
            haloLayer.path = UIBezierPath(arcCenter: CGPoint(x: thumbX, y: frame.midY), radius: radius, startAngle: 0, endAngle: 2 * .pi, clockwise: true).cgPath
            haloLayer.strokeColor = hot
            haloLayer.opacity = Float(top * 0.6 * (1 - wave))
        } else {
            haloLayer.opacity = 0
        }
        CATransaction.commit()
    }
}

/// The pill track: the empty track with dots for the steps ahead, the heat
/// fill to the thumb with its sheen and sparks, and the white thumb inset in
/// the track. Paint only; the slider drives it.
private final class EffortTrackView: UIView {
    var count = 0
    var fraction: CGFloat = 0
    var motion = EffortScale.Motion()
    /// VoiceOver swipes: +1 up, -1 down.
    var onAdjust: ((Int) -> Void)?

    private static let trackColor = Palette.dynamic(0x27272C, 0xE8E8EA, alpha: 0.1)
    private static let tickColor = Palette.dynamic(0x62626A, 0xA9A9AE, alpha: 0.45)
    private static let space = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()
    private static let fillLocations: [CGFloat] = (0...6).map { CGFloat($0) / 6 }
    /// Clear → white → clear; alpha set per draw.
    private static let sheen = CGGradient(colorsSpace: space, colors: [UIColor(white: 1, alpha: 0).cgColor, UIColor.white.cgColor, UIColor(white: 1, alpha: 0).cgColor] as CFArray, locations: [0, 0.5, 1])
    /// Clear → white, for the streaks; alpha set per spark.
    private static let trail = CGGradient(colorsSpace: space, colors: [UIColor(white: 1, alpha: 0).cgColor, UIColor.white.cgColor] as CFArray, locations: [0, 1])

    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = false
        backgroundColor = .clear
        contentMode = .redraw
    }

    required init?(coder: NSCoder) { fatalError() }

    override func accessibilityIncrement() { onAdjust?(1) }
    override func accessibilityDecrement() { onAdjust?(-1) }

    /// A horizontal drag on the track is the slider's, not an enclosing
    /// scroll view's or sheet's (like UISlider).
    override func gestureRecognizerShouldBegin(_ g: UIGestureRecognizer) -> Bool {
        if let pan = g as? UIPanGestureRecognizer, g.view !== self {
            let v = pan.velocity(in: self)
            if abs(v.x) > abs(v.y) { return false }
        }
        return super.gestureRecognizerShouldBegin(g)
    }

    override func draw(_ rect: CGRect) {
        guard let ctx = UIGraphicsGetCurrentContext(), bounds.width > bounds.height, bounds.height > 0 else { return }
        let w = bounds.width
        let h = bounds.height
        let r = h / 2
        let run = w - 2 * r
        let f = min(1, max(0, fraction))
        let thumbX = r + f * run
        let fillWidth = min(w, thumbX + r)
        let traits = traitCollection

        Self.trackColor.resolvedColor(with: traits).setFill()
        UIBezierPath(roundedRect: bounds, cornerRadius: r).fill()

        // Steps not yet reached, as dots in the empty track.
        if count > 1 {
            Self.tickColor.resolvedColor(with: traits).setFill()
            let dot: CGFloat = 2.5
            for i in 0..<count {
                let x = r + CGFloat(i) / CGFloat(count - 1) * run
                if x > thumbX + r { ctx.fillEllipse(in: CGRect(x: x - dot, y: r - dot, width: dot * 2, height: dot * 2)) }
            }
        }

        ctx.saveGState()
        UIBezierPath(roundedRect: CGRect(x: 0, y: 0, width: fillWidth, height: h), cornerRadius: r).addClip()
        // The band of heat ending at the level.
        let band = EffortScale.fillBand(fraction: f).map { $0.color.cgColor }
        if let gradient = CGGradient(colorsSpace: Self.space, colors: band as CFArray, locations: Self.fillLocations) {
            ctx.drawLinearGradient(gradient, start: .zero, end: CGPoint(x: fillWidth, y: 0), options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
        }

        // A sheen sweeping through, quicker and brighter with the effort.
        if let sheen = Self.sheen {
            let width = h * 2.2
            let x = -width + CGFloat(motion.sweep) * (fillWidth + 2 * width)
            ctx.setAlpha(0.10 + 0.22 * f)
            ctx.drawLinearGradient(sheen, start: CGPoint(x: x, y: 0), end: CGPoint(x: x + width, y: 0), options: [])
            ctx.setAlpha(1)
        }

        // Sparks: denser and quicker with the effort, streaking at the high end.
        let streak = EffortScale.streak(fraction: f)
        for s in EffortScale.sparks.prefix(EffortScale.visibleSparks(fraction: f)) {
            let sx = EffortScale.sparkX(s, drift: motion.drift) * fillWidth
            let sy = s.y * h
            let alpha = EffortScale.sparkAlpha(s, breath: motion.breath, fraction: f)
            if streak > 1, let trail = Self.trail {
                let start = CGPoint(x: sx - streak * s.speed, y: sy)
                ctx.saveGState()
                ctx.setLineWidth(s.radius)
                ctx.setLineCap(.round)
                ctx.move(to: start)
                ctx.addLine(to: CGPoint(x: sx, y: sy))
                ctx.replacePathWithStrokedPath()
                ctx.clip()
                ctx.setAlpha(alpha * 0.6)
                ctx.drawLinearGradient(trail, start: start, end: CGPoint(x: sx, y: sy), options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
                ctx.restoreGState()
            }
            UIColor(white: 1, alpha: alpha).setFill()
            ctx.fillEllipse(in: CGRect(x: sx - s.radius, y: sy - s.radius, width: s.radius * 2, height: s.radius * 2))
        }
        ctx.restoreGState()

        // The thumb: white, inset in the track, with a soft shadow.
        let tr = r - 4
        ctx.saveGState()
        ctx.setShadow(offset: CGSize(width: 0, height: 1), blur: 4, color: UIColor(white: 0, alpha: 0.22).cgColor)
        UIColor.white.setFill()
        ctx.fillEllipse(in: CGRect(x: thumbX - tr, y: r - tr, width: tr * 2, height: tr * 2))
        ctx.restoreGState()
    }
}

/// Only horizontal drags move the thumb; vertical ones go to the sheet.
private final class HorizontalPanDelegate: NSObject, UIGestureRecognizerDelegate {
    func gestureRecognizerShouldBegin(_ g: UIGestureRecognizer) -> Bool {
        guard let pan = g as? UIPanGestureRecognizer else { return true }
        let v = pan.velocity(in: pan.view)
        return abs(v.x) >= abs(v.y)
    }
}

/// CADisplayLink retains its target; the proxy keeps the slider releasable.
private final class EffortLinkProxy: NSObject {
    weak var slider: EffortSlider?
    init(_ slider: EffortSlider) { self.slider = slider }
    @objc func tick(_ link: CADisplayLink) { MainActor.assumeIsolated { slider?.tick(link) } }
}
