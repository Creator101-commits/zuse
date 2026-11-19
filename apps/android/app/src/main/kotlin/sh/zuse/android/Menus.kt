package sh.zuse.android

import androidx.compose.animation.core.*
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties

/** Items are laid out in a column; dividers and labels group them. */
class ZuseMenuScope internal constructor(internal val dismiss: () -> Unit)

private val menuShape = RoundedCornerShape(16.dp)

/**
 * A compact popover anchored to its parent: a rounded card with a soft shadow that grows from the
 * corner it is attached to. Replaces the platform dropdown, whose square, dense look doesn't fit.
 */
@Composable
fun ZuseMenu(
    expanded: Boolean,
    onDismiss: () -> Unit,
    alignEnd: Boolean = true,
    minWidth: Dp = 200.dp,
    content: @Composable ZuseMenuScope.() -> Unit,
) {
    val state = remember { MutableTransitionState(false) }
    state.targetState = expanded
    val haptics = LocalHapticFeedback.current
    LaunchedEffect(expanded) { if (expanded) haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove) }
    if (!state.currentState && !state.targetState) return
    val density = LocalDensity.current
    var above by remember { mutableStateOf(false) }
    val provider =
        remember(alignEnd, density) {
            AnchoredMenu(alignEnd, with(density) { 6.dp.roundToPx() }, with(density) { 12.dp.roundToPx() }) {
                above = it
            }
        }
    Popup(
        popupPositionProvider = provider,
        onDismissRequest = onDismiss,
        properties = PopupProperties(focusable = true),
    ) {
        val transition = rememberTransition(state, label = "menu")
        val scale by
            transition.animateFloat(
                { if (targetState) tween(160, easing = FastOutSlowInEasing) else tween(110) },
                label = "scale",
            ) {
                if (it) 1f else .92f
            }
        val alpha by
            transition.animateFloat({ tween(if (targetState) 140 else 90) }, label = "alpha") {
                if (it) 1f else 0f
            }
        val dark = isSystemInDarkTheme()
        Surface(
            shape = menuShape,
            color = if (dark) Color(0xFF232323) else Color.White,
            contentColor = MaterialTheme.colorScheme.onSurface,
            shadowElevation = 12.dp,
            border =
                BorderStroke(
                    0.5.dp,
                    MaterialTheme.colorScheme.outlineVariant.copy(alpha = if (dark) .6f else .45f),
                ),
            modifier =
                Modifier.graphicsLayer {
                    scaleX = scale
                    scaleY = scale
                    this.alpha = alpha
                    transformOrigin =
                        TransformOrigin(if (alignEnd) 1f else 0f, if (above) 1f else 0f)
                },
        ) {
            Column(
                Modifier.width(IntrinsicSize.Max)
                    .widthIn(min = minWidth, max = 300.dp)
                    .heightIn(max = 420.dp)
                    .verticalScroll(rememberScrollState())
                    .padding(vertical = 6.dp)
            ) {
                ZuseMenuScope(onDismiss).content()
            }
        }
    }
}

/** One menu row. [selected] shows a check; [destructive] colors it as an error. */
@Composable
fun ZuseMenuScope.MenuItem(
    text: String,
    icon: ImageVector? = null,
    detail: String? = null,
    selected: Boolean = false,
    destructive: Boolean = false,
    enabled: Boolean = true,
    onClick: () -> Unit,
) {
    val tint =
        when {
            !enabled -> MaterialTheme.colorScheme.onSurface.copy(alpha = .38f)
            destructive -> MaterialTheme.colorScheme.error
            else -> MaterialTheme.colorScheme.onSurface
        }
    Row(
        Modifier.fillMaxWidth()
            .padding(horizontal = 6.dp)
            .clip(RoundedCornerShape(10.dp))
            .clickable(enabled = enabled, role = Role.Button) {
                dismiss()
                onClick()
            }
            .semantics { if (selected) this.selected = true }
            .heightIn(min = 44.dp)
            .padding(horizontal = 10.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (icon != null)
            Icon(
                icon,
                null,
                Modifier.size(18.dp),
                tint = if (destructive || !enabled) tint else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        Column(Modifier.weight(1f)) {
            Text(
                text,
                style = MaterialTheme.typography.bodyMedium.copy(fontSize = 15.sp),
                color = tint,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (detail != null)
                Text(
                    detail,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
        }
        if (selected)
            Icon(Icons.Outlined.Check, "Selected", Modifier.size(18.dp), tint = MaterialTheme.colorScheme.primary)
    }
}

@Composable
fun ZuseMenuScope.MenuDivider() {
    HorizontalDivider(
        Modifier.padding(vertical = 6.dp),
        thickness = 0.5.dp,
        color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = .7f),
    )
}

@Composable
fun ZuseMenuScope.MenuLabel(text: String) {
    Text(
        text,
        Modifier.padding(start = 16.dp, end = 16.dp, top = 6.dp, bottom = 4.dp).semantics {
            role = Role.Image
        },
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

/** Below the anchor, aligned to its start or end; above it when there's no room below. */
private class AnchoredMenu(
    private val alignEnd: Boolean,
    private val gap: Int,
    private val margin: Int,
    private val placedAbove: (Boolean) -> Unit,
) : PopupPositionProvider {
    override fun calculatePosition(
        anchorBounds: IntRect,
        windowSize: IntSize,
        layoutDirection: LayoutDirection,
        popupContentSize: IntSize,
    ): IntOffset {
        val end = alignEnd != (layoutDirection == LayoutDirection.Rtl)
        val x =
            (if (end) anchorBounds.right - popupContentSize.width else anchorBounds.left)
                .coerceIn(margin, (windowSize.width - popupContentSize.width - margin).coerceAtLeast(margin))
        val below = anchorBounds.bottom + gap
        val fitsBelow = below + popupContentSize.height <= windowSize.height - margin
        val y =
            if (fitsBelow) below
            else (anchorBounds.top - gap - popupContentSize.height).coerceAtLeast(margin)
        placedAbove(!fitsBelow)
        return IntOffset(x, y)
    }
}
