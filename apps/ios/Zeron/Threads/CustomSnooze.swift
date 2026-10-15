import UIKit

/// The snooze menus' "Custom…": a native alert asking for a whole number,
/// then the unit as the confirming button (Minutes / Hours / Days). A unit
/// stays disabled until the number is positive and the span fits the core's
/// `UInt32` seconds.
enum CustomSnooze {
    static let units: [(title: String, seconds: UInt32)] = [
        ("Minutes", 60),
        ("Hours", 60 * 60),
        ("Days", 24 * 60 * 60),
    ]

    /// Seconds for `text` in `unit`, or nil when not a positive whole number
    /// or past `UInt32.max`.
    static func seconds(_ text: String, unit: UInt32) -> UInt32? {
        guard let amount = UInt32(text.trimmingCharacters(in: .whitespaces)), amount > 0 else { return nil }
        let (total, overflow) = amount.multipliedReportingOverflow(by: unit)
        return overflow ? nil : total
    }

    /// "90 Minutes", "1 Hour": the toast reads "Snoozed for 90 minutes".
    static func title(_ text: String, unit: String) -> String {
        let amount = text.trimmingCharacters(in: .whitespaces)
        return "\(amount) \(amount == "1" ? String(unit.dropLast()) : unit)"
    }

    @MainActor
    static func present(from host: UIViewController, pick: @escaping (_ seconds: UInt32, _ title: String) -> Void) {
        let alert = UIAlertController(title: "Snooze For…", message: "Enter how long, then choose the unit.", preferredStyle: .alert)
        var actions: [(UIAlertAction, UInt32)] = []
        alert.addTextField { field in
            field.keyboardType = .numberPad
            field.placeholder = "Amount"
            field.accessibilityIdentifier = "custom-snooze-amount"
            field.addAction(UIAction { [weak field] _ in
                let text = field?.text ?? ""
                for (action, unit) in actions { action.isEnabled = seconds(text, unit: unit) != nil }
            }, for: .editingChanged)
        }
        for unit in units {
            let action = UIAlertAction(title: unit.title, style: .default) { [weak alert] _ in
                let text = alert?.textFields?.first?.text ?? ""
                guard let total = seconds(text, unit: unit.seconds) else { return }
                pick(total, title(text, unit: unit.title))
            }
            action.isEnabled = false
            actions.append((action, unit.seconds))
            alert.addAction(action)
        }
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
        // Reached from the swipe's action sheet or a menu on its way out:
        // present only once that controller is gone, so UIKit never refuses
        // an alert over one mid-dismissal.
        let show = { [weak host] in host?.present(alert, animated: true) }
        if let outgoing = host.presentedViewController {
            // Already leaving: follow its transition. (`animate` is false when
            // nothing could be queued.)
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
