import UIKit

/// Past the 15m/1h/4h/1d presets the question is *when it comes back*, not a
/// duration: a few contextual wake times (this evening, tomorrow morning, next
/// week) and the native date & time picker. Every choice becomes the core's
/// duration in whole minutes, bounded by its `UInt32` seconds.
enum SnoozeUntil {
    struct Suggestion: Equatable {
        let title: String
        let date: Date
    }

    static let eveningHour = 18
    static let morningHour = 9

    /// This evening (while it is more than an hour away), tomorrow morning,
    /// and next Monday morning unless that is tomorrow.
    static func suggestions(now: Date, calendar: Calendar = .current) -> [Suggestion] {
        var out: [Suggestion] = []
        let today = calendar.startOfDay(for: now)
        if let evening = calendar.date(bySettingHour: eveningHour, minute: 0, second: 0, of: now),
           evening.timeIntervalSince(now) > 60 * 60 {
            out.append(Suggestion(title: "This Evening", date: evening))
        }
        guard let tomorrow = calendar.date(byAdding: .day, value: 1, to: today),
              let morning = calendar.date(bySettingHour: morningHour, minute: 0, second: 0, of: tomorrow)
        else { return out }
        out.append(Suggestion(title: "Tomorrow Morning", date: morning))
        if let monday = calendar.nextDate(after: tomorrow, matching: DateComponents(hour: morningHour, minute: 0, weekday: 2), matchingPolicy: .nextTime),
           !calendar.isDate(monday, inSameDayAs: morning) {
            out.append(Suggestion(title: "Next Week", date: monday))
        }
        return out
    }

    /// The core's duration for waking at `date`: whole minutes, rounded up, at
    /// least one; nil when `date` has passed or lies beyond `UInt32` seconds.
    static func seconds(until date: Date, now: Date) -> UInt32? {
        let span = date.timeIntervalSince(now)
        guard span > 0 else { return nil }
        let minutes = (span / 60).rounded(.up)
        let total = max(60, minutes * 60)
        return total <= Double(UInt32.max) ? UInt32(total) : nil
    }

    /// "6:00 PM" today, "Thu 9:00 AM" within the week, "Oct 21, 9:00 AM"
    /// beyond, with the year once it differs. Follows the device's 12/24-hour
    /// setting.
    static func label(_ date: Date, now: Date, calendar: Calendar = .current, locale: Locale = .current) -> String {
        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.timeZone = calendar.timeZone
        formatter.locale = locale
        let days = calendar.dateComponents([.day], from: calendar.startOfDay(for: now), to: calendar.startOfDay(for: date)).day ?? 0
        let template = calendar.component(.year, from: date) != calendar.component(.year, from: now) ? "yMMMdjmm"
            : days == 0 ? "jmm" : days < 7 ? "EEEjmm" : "MMMdjmm"
        formatter.setLocalizedDateFormatFromTemplate(template)
        return formatter.string(from: date)
    }

    /// The menu tail after the presets: an inline "until" section with each
    /// wake time as a subtitle, then "Pick Date & Time…".
    @MainActor
    static func menuElements(now: Date = Date(), snooze: @escaping (Date) -> Void, pickDate: @escaping () -> Void) -> [UIMenuElement] {
        let until = suggestions(now: now).map { s in
            UIAction(title: s.title, subtitle: label(s.date, now: now), image: UIImage(systemName: "moon.zzz")) { _ in snooze(s.date) }
        }
        return [
            UIMenu(options: .displayInline, children: until),
            UIAction(title: "Pick Date & Time…", image: UIImage(systemName: "calendar.badge.clock")) { _ in pickDate() },
        ]
    }

    /// Present the picker sheet from `host`, once anything still on its way
    /// out (the swipe's action sheet, a closing menu) has gone. `snooze`
    /// reports whether the write succeeded; the sheet closes either way.
    @MainActor
    static func pickDate(from host: UIViewController, snooze: @escaping (Date) -> Void) {
        let picker = SnoozeDatePickerController(snooze: snooze)
        let show = { [weak host] in host?.present(picker, animated: true) }
        if let outgoing = host.presentedViewController {
            if outgoing.isBeingDismissed,
               outgoing.transitionCoordinator?.animate(alongsideTransition: nil, completion: { _ in show() }) == true {
                return
            }
            // An action handler can run before UIKit starts the sheet's own
            // dismissal: dismiss it here and present from its completion.
            if outgoing is UIAlertController, !outgoing.isBeingDismissed {
                outgoing.dismiss(animated: true, completion: show)
                return
            }
        }
        if host.transitionCoordinator?.animate(alongsideTransition: nil, completion: { _ in show() }) == true { return }
        show()
    }
}

/// A small sheet: the inline calendar with its time row, and one button that
/// names exactly when the session comes back.
final class SnoozeDatePickerController: UIViewController {
    private let snooze: (Date) -> Void
    private let picker = UIDatePicker()
    private let confirm = UIButton(configuration: .borderedProminent())

    init(snooze: @escaping (Date) -> Void) {
        self.snooze = snooze
        super.init(nibName: nil, bundle: nil)
        modalPresentationStyle = .pageSheet
        if let sheet = sheetPresentationController {
            sheet.detents = [.custom(identifier: .init("snooze-picker")) { _ in 560 }, .large()]
            sheet.prefersGrabberVisible = true
            sheet.preferredCornerRadius = 28
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        let now = Date()
        let title = UILabel()
        title.text = "Snooze Until"
        title.font = Fonts.ui(.sansSemibold, TypeScale.size(17))
        title.textColor = Palette.text
        title.accessibilityTraits = .header

        picker.datePickerMode = .dateAndTime
        picker.preferredDatePickerStyle = .inline
        picker.minuteInterval = 5
        picker.tintColor = Palette.accent
        picker.minimumDate = now.addingTimeInterval(60)
        picker.maximumDate = now.addingTimeInterval(TimeInterval(UInt32.max))
        // Start where most custom snoozes land: tomorrow morning.
        picker.date = SnoozeUntil.suggestions(now: now).first { $0.title == "Tomorrow Morning" }?.date ?? now.addingTimeInterval(3600)
        picker.accessibilityIdentifier = "snooze-until-picker"
        picker.addAction(UIAction { [weak self] _ in self?.refresh() }, for: .valueChanged)

        confirm.configuration?.baseBackgroundColor = Palette.accent
        confirm.configuration?.cornerStyle = .capsule
        confirm.accessibilityIdentifier = "snooze-until-confirm"
        confirm.addAction(UIAction { [weak self] _ in self?.done() }, for: .primaryActionTriggered)

        let stack = UIStackView(arrangedSubviews: [title, picker, confirm])
        stack.axis = .vertical
        stack.spacing = 12
        stack.setCustomSpacing(4, after: title)
        stack.translatesAutoresizingMaskIntoConstraints = false
        // Large text and short landscape sheets scroll rather than clip.
        let scroll = UIScrollView()
        scroll.alwaysBounceVertical = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.addSubview(stack)
        view.addSubview(scroll)
        let content = scroll.contentLayoutGuide
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            stack.topAnchor.constraint(equalTo: content.topAnchor, constant: 24),
            stack.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -24),
            stack.leadingAnchor.constraint(equalTo: view.layoutMarginsGuide.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: view.layoutMarginsGuide.trailingAnchor),
            confirm.heightAnchor.constraint(greaterThanOrEqualToConstant: 50),
        ])
        title.adjustsFontForContentSizeCategory = true
        refresh()
    }

    private func refresh() {
        let now = Date()
        let valid = SnoozeUntil.seconds(until: picker.date, now: now) != nil
        var config = confirm.configuration
        config?.title = valid ? "Snooze until \(SnoozeUntil.label(picker.date, now: now))" : "Pick a later time"
        confirm.configuration = config
        confirm.isEnabled = valid
    }

    /// Re-checked at the tap: a sheet left open can outlive its choice, and
    /// then it stays up asking for a later time instead of failing quietly.
    private func done() {
        let date = picker.date
        guard SnoozeUntil.seconds(until: date, now: Date()) != nil else {
            picker.minimumDate = Date().addingTimeInterval(60)
            refresh()
            UINotificationFeedbackGenerator().notificationOccurred(.warning)
            return
        }
        dismiss(animated: true) { [snooze] in snooze(date) }
    }
}
