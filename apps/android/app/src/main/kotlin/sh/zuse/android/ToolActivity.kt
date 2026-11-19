package sh.zuse.android

import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.serialization.json.*
import sh.zuse.core.*

private const val VISIBLE_STEPS = 12
private const val PREVIEW_ICONS = 5

private fun ToolIcon.vector(): ImageVector =
    when (this) {
        ToolIcon.Terminal -> Icons.Outlined.Terminal
        ToolIcon.Read -> Icons.Outlined.Description
        ToolIcon.Edit -> Icons.Outlined.Edit
        ToolIcon.Search -> Icons.Outlined.Search
        ToolIcon.Web -> Icons.Outlined.Language
        ToolIcon.Tasks -> Icons.Outlined.Checklist
        ToolIcon.Plugin -> Icons.Outlined.Extension
        ToolIcon.Agent -> Icons.Outlined.Groups
        ToolIcon.Thinking -> Icons.Outlined.Psychology
        ToolIcon.Other -> Icons.Outlined.Build
    }

/**
 * Activity folds behind one line: while it runs that line names the step in progress, and once it
 * settles it summarizes the run, so the answer is what a reader sees. Tapping opens every step.
 */
@Composable
fun ToolActivityGroup(group: TranscriptGroup, live: Boolean) {
    var choice by rememberSaveable(group.id) { mutableStateOf<Boolean?>(null) }
    val expanded = choice ?: false
    var showAll by rememberSaveable(group.id) { mutableStateOf(false) }
    val title =
        if (live) ToolPresentation.liveTitle(group.rows) else ToolPresentation.summary(group.rows)
    // Like desktop: a few distinct tool icons follow the summary, in order of first use.
    val icons = remember(group.rows) {
        group.rows.map { if (it.isThinking) ToolIcon.Thinking else ToolPresentation.step(it.message).icon }
            .distinct()
            .take(PREVIEW_ICONS)
    }
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    Column(Modifier.fillMaxWidth().animateContentSize(tween(160))) {
        // No disclosure arrow, matching desktop: the whole line is the toggle. It stays quiet even
        // when a step failed; failures show on the step itself and in the summary's count.
        Row(
            Modifier.fillMaxWidth()
                .heightIn(min = 36.dp)
                .clickable(
                    interactionSource = remember { androidx.compose.foundation.interaction.MutableInteractionSource() },
                    indication = null,
                ) {
                    choice = !expanded
                }
                .semantics {
                    role = Role.Button
                    stateDescription = if (expanded) "Expanded" else "Collapsed"
                },
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Row(
                Modifier.weight(1f, fill = false),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                ActivityTitle(title, live, Modifier.weight(1f, fill = false))
                if (live && group.rows.size > 1)
                    Text(
                        "· ${group.rows.size} steps",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = .7f),
                        maxLines = 1,
                    )
            }
            if (!live)
                icons.forEach { icon ->
                    Icon(icon.vector(), null, Modifier.size(13.dp), tint = muted.copy(alpha = .7f))
                }
        }
        if (expanded) {
            val hidden = if (showAll) 0 else (group.rows.size - VISIBLE_STEPS).coerceAtLeast(0)
            val rail = MaterialTheme.colorScheme.outlineVariant
            Column(
                Modifier.padding(start = 7.dp).fillMaxWidth().drawBehind {
                    drawLine(rail, Offset(0f, 0f), Offset(0f, size.height), 1.dp.toPx())
                }
            ) {
                if (hidden > 0)
                    TextButton(
                        onClick = { showAll = true },
                        modifier = Modifier.padding(start = 8.dp).height(30.dp),
                        contentPadding = PaddingValues(horizontal = 8.dp),
                    ) {
                        Text("Show $hidden earlier", style = MaterialTheme.typography.labelSmall)
                    }
                group.rows.drop(hidden).forEach { row -> key(row.id) { ToolStepRow(row) } }
            }
        }
    }
}

@Composable
private fun ActivityTitle(text: String, live: Boolean, modifier: Modifier) {
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    val style =
        MaterialTheme.typography.labelLarge.copy(fontWeight = FontWeight.Normal, fontSize = 14.sp)
    if (!live) {
        Text(
            text,
            modifier,
            style = style,
            color = muted,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        return
    }
    val peak = MaterialTheme.colorScheme.onSurface
    val sweep by
        rememberInfiniteTransition(label = "activity title")
            .animateFloat(
                -300f,
                900f,
                infiniteRepeatable(tween(1800, easing = LinearEasing)),
                label = "sweep",
            )
    Text(
        text,
        modifier,
        style =
            style.copy(
                brush =
                    Brush.linearGradient(
                        listOf(muted, peak, muted),
                        Offset(sweep, 0f),
                        Offset(sweep + 300f, 0f),
                    )
            ),
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
    )
}

@Composable
private fun ToolStepRow(row: TranscriptRow) {
    var open by rememberSaveable(row.id) { mutableStateOf(false) }
    val step = remember(row) { ToolPresentation.step(row.message) }
    val body = remember(row) { stepBody(row) }
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    Column(Modifier.padding(start = 10.dp).fillMaxWidth()) {
        Row(
            Modifier.fillMaxWidth()
                .heightIn(min = 32.dp)
                .then(if (body != null) Modifier.clickable { open = !open } else Modifier),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(7.dp),
        ) {
            if (row.running) CircularProgressIndicator(Modifier.size(13.dp), strokeWidth = 1.5.dp)
            else
                Icon(step.icon.vector(), null, Modifier.size(15.dp), tint = muted)
            Text(
                step.label,
                style = MaterialTheme.typography.labelMedium,
                fontWeight = FontWeight.Medium,
                color = MaterialTheme.colorScheme.onSurface,
            )
            if (step.detail.isNotEmpty())
                Text(
                    step.detail,
                    Modifier.weight(1f),
                    style = MaterialTheme.typography.bodySmall.copy(fontSize = 12.sp),
                    fontFamily = FontFamily.Monospace,
                    color = if (row.failed) MaterialTheme.colorScheme.error else muted,
                    maxLines = 1,
                    overflow = TextOverflow.MiddleEllipsis,
                )
        }
        if (open && body != null)
            Surface(
                shape = RoundedCornerShape(8.dp),
                color = MaterialTheme.colorScheme.surfaceContainer,
                modifier = Modifier.fillMaxWidth().padding(top = 2.dp, bottom = 6.dp),
            ) {
                SelectionContainer {
                    Text(
                        body,
                        Modifier.heightIn(max = 260.dp)
                            .verticalScroll(rememberScrollState())
                            .horizontalScroll(rememberScrollState())
                            .padding(10.dp),
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 17.sp),
                        fontFamily = FontFamily.Monospace,
                    )
                }
            }
    }
}

/** What opening a step shows; `null` when there is nothing beyond its one line. */
private fun stepBody(row: TranscriptRow): String? {
    val content = row.message.content
    if (row.isThinking)
        return if (content.flag("redacted")) "Thinking redacted"
        else row.message.text.takeIf { it.isNotBlank() }
    val output = (row.result?.content ?: content).field("output")
    return when (output) {
        is JsonPrimitive -> output.contentOrNull
        is JsonNull -> null
        else -> output.toString()
    }?.takeIf { it.isNotBlank() }
}
