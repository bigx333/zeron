package sh.zeron.android.voice

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription

/** One source per button; leaving the screen or cancelling a gesture releases it. */
@Composable
internal fun Modifier.holdToTalk(controller: VoiceController, enabled: Boolean, muted: Boolean): Modifier {
    val source = remember { Any() }
    val haptics = LocalHapticFeedback.current
    DisposableEffect(controller, source) {
        onDispose { controller.hold(false, source) }
    }
    return this
        .semantics { if (enabled) stateDescription = if (muted) "Microphone off" else "Transmitting" }
        .pointerInput(controller, enabled) {
            if (!enabled) return@pointerInput
            try {
                awaitEachGesture {
                    // Own this press in hold mode. Letting IconButton handle
                    // it as a click would consume moves and cancel the hold.
                    awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial).consume()
                    controller.hold(true, source)
                    haptics.performHapticFeedback(HapticFeedbackType.GestureThresholdActivate)
                    try {
                        waitForUpOrCancellation(pass = PointerEventPass.Initial)?.consume()
                    } finally {
                        controller.hold(false, source)
                        haptics.performHapticFeedback(HapticFeedbackType.ToggleOff)
                    }
                }
            } finally {
                controller.hold(false, source)
            }
        }
}
