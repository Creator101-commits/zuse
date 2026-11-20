// Pure helpers that turn changelog releases into cover cards and collage
// panels. Everything here is deterministic so server and client renders agree.

export type ChangeSection = { title: string; items: string[] };
export type ChangeRelease = {
	version: string;
	date: string | null;
	sections: ChangeSection[];
};

export type Shot = {
	id: string;
	src: string;
	/** CSS object-position used when the shot is cropped into a frame. */
	focus: string;
	/** Keywords from changelog text that this screenshot illustrates. */
	match: RegExp;
};

const shot = (id: string, match: RegExp, focus = "50% 0%"): Shot => ({
	id,
	src: `/changelog/shots/${id}.webp`,
	focus,
	match,
});

// Real product screenshots, most specific matches first.
export const shots: readonly Shot[] = [
	shot("checks-failed", /\b(fail|failing|crash|error|broken|oom|exhaust)/i, "30% 20%"),
	shot("checks", /\b(pull request|pr|checks?|ci|commit|merge|branch)\b/i, "30% 20%"),
	shot("approval", /\b(approv|allow|full access|sandbox|write file)/i, "20% 60%"),
	shot("permissions", /\b(permission|rules?|policy|secret|credential)/i, "30% 40%"),
	shot("simulator", /\b(simulator|ios|android|phone|mobile app|device)/i, "50% 10%"),
	shot("devices", /\b(computer|machine|runtime|remote|cloud|container)/i, "20% 10%"),
	shot("plugins", /\b(mcp|plugin|provider|command|catalog|tool)/i, "50% 0%"),
	shot("connect", /\b(sign[- ]in|account|connect|slack|auth|billing|subscription|link)/i, "50% 0%"),
	shot("review", /\b(diff|review|changes|markdown|mermaid|table)/i, "20% 30%"),
	shot("composer", /\b(composer|model picker|model|slash|attachment|prompt|reasoning)/i, "0% 50%"),
	shot("files", /\b(file|folder|import|browser|zoom|page)/i, "30% 0%"),
	shot("conversation", /\b(chat|message|thread|question|conversation|queue|steer|reply)/i, "30% 20%"),
	shot("phone-chat", /\b(notification|mobile|on the go)/i, "50% 0%"),
	shot("simulator-menu", /\b(stop|control|menu|shortcut)/i, "60% 0%"),
	shot("workspace", /\b(workspace|project|sidebar|window|layout|setup|settings|agent)/i, "0% 0%"),
	shot("observatory", /\b(release|update|install|download|linux|macos|windows|installer)/i, "50% 40%"),
	shot("pegasus", /\b(fast|speed|faster|instant|launch)/i, "50% 40%"),
	shot("handoff", /\b(handoff|hand off|share|team|zapper)/i, "50% 50%"),
];

/**
 * Picks the screenshot that best illustrates a change, preferring shots not
 * yet used in the same release so a collage never repeats an image early.
 */
export function shotFor(text: string, used: ReadonlySet<string> = new Set()): Shot {
	const plain = plainText(text);
	const fresh = shots.filter((s) => !used.has(s.id));
	const pool = fresh.length > 0 ? fresh : shots;
	return pool.find((s) => s.match.test(plain)) ?? (pool[hashString(plain) % pool.length] as Shot);
}

export type SectionKind = "added" | "changed" | "fixed" | "other";

export function sectionKind(title: string): SectionKind {
	const t = title.toLowerCase();
	if (t.startsWith("add") || t.startsWith("new")) return "added";
	if (t.startsWith("change") || t.startsWith("improv")) return "changed";
	if (t.startsWith("fix")) return "fixed";
	return "other";
}

export function sectionTag(title: string): string {
	switch (sectionKind(title)) {
		case "added":
			return "New";
		case "changed":
			return "Improved";
		case "fixed":
			return "Fixed";
		default:
			return title;
	}
}

export function hashString(value: string): number {
	let hash = 2166136261;
	for (let i = 0; i < value.length; i++) {
		hash ^= value.charCodeAt(i);
		hash = Math.imul(hash, 16777619);
	}
	return hash >>> 0;
}

/** Removes inline Markdown so text can be used as a headline. */
export function plainText(text: string): string {
	return text
		.replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
		.replace(/\*\*([^*]+)\*\*/g, "$1")
		.replace(/`([^`]+)`/g, "$1")
		.replace(/\s+/g, " ")
		.trim();
}

const leadingVerbs =
	/^(add|adds|added|show|shows|keep|keeps|make|makes|let|lets|use|uses|render|renders|support|supports|allow|allows|improve|improves|improved|refine|refined|correct|avoid|preserve|choose|a|an|the)\s+/i;

const danglingWords =
	/^(to|and|or|the|a|an|of|in|on|for|as|with|from|by|at|into|that|when|so|its|your|their|is|are|be|now|all|new)$/i;

/**
 * Short headline for a change item: the first clause, trimmed to a handful of
 * words without ending on a filler word.
 */
export function headline(text: string, maxWords = 5): string {
	let clause = plainText(text).split(/(?<=[a-z0-9)])[.:;,]\s|\s[–—-]\s|\(/i)[0] ?? "";
	clause = clause.replace(/[.:;,]+$/, "");
	let words = clause.split(" ").filter(Boolean);
	while (words.length > 2 && leadingVerbs.test(`${words[0]} `)) {
		words = words.slice(1);
	}
	let cut = words.slice(0, maxWords);
	while (cut.length > 2 && danglingWords.test(cut[cut.length - 1] ?? "")) {
		cut = cut.slice(0, -1);
	}
	const result = cut.join(" ");
	return result.charAt(0).toUpperCase() + result.slice(1);
}

export type PanelShape = { cols: number; rows: number };

// Card rhythm: a wide lead card, a row of three, then a pair.
const shapeCycle: readonly PanelShape[] = [
	{ cols: 8, rows: 1 },
	{ cols: 4, rows: 1 },
	{ cols: 4, rows: 1 },
	{ cols: 4, rows: 1 },
	{ cols: 4, rows: 1 },
	{ cols: 6, rows: 1 },
	{ cols: 6, rows: 1 },
];

export type CollagePanel = {
	id: string;
	text: string;
	headline: string;
	section: string;
	kind: SectionKind;
	shot: Shot;
	shape: PanelShape;
	/** Wide hero frame that gets the big headline. */
	large: boolean;
};

export function collagePanels(release: ChangeRelease): CollagePanel[] {
	const panels: CollagePanel[] = [];
	const used = new Set<string>();
	for (const section of release.sections) {
		const kind = sectionKind(section.title);
		for (const text of section.items) {
			const index = panels.length;
			const pick = shotFor(text, used);
			used.add(pick.id);
			const shape = shapeCycle[index % shapeCycle.length] as PanelShape;
			panels.push({
				id: `${release.version}-${index}`,
				text,
				headline: headline(text),
				section: section.title,
				kind,
				shot: pick,
				shape,
				large: shape.cols >= 8,
			});
		}
	}
	return panels;
}

export type ReleaseCover = {
	version: string;
	date: string | null;
	title: string;
	/** Nine frames for the cover collage, in reading order. */
	shots: Shot[];
	changes: number;
	preview: boolean;
};

export function releaseCover(release: ChangeRelease): ReleaseCover {
	const items = release.sections.flatMap((section) => section.items);
	const lead =
		release.sections.find((s) => sectionKind(s.title) === "added")?.items[0] ??
		items[0] ??
		release.version;
	return {
		version: release.version,
		date: release.date,
		title: headline(lead, 4),
		shots: coverShots(release.version, items),
		changes: items.length,
		preview: /preview|beta|rc/i.test(release.version),
	};
}

function coverShots(version: string, items: string[]): Shot[] {
	const picked: Shot[] = [];
	const used = new Set<string>();
	for (const item of items) {
		const pick = shotFor(item, used);
		if (used.has(pick.id)) break;
		used.add(pick.id);
		picked.push(pick);
		if (picked.length === 9) return picked;
	}
	// Pad with the remaining shots in a per-release order.
	const rest = shots.filter((s) => !used.has(s.id));
	const offset = hashString(version);
	for (let i = 0; picked.length < 9 && i < rest.length; i++) {
		picked.push(rest[(offset + i) % rest.length] as Shot);
	}
	return picked;
}

export function formatReleaseDate(date: string | null): string {
	if (!date) return "Unreleased";
	const parsed = new Date(`${date}T00:00:00Z`);
	if (Number.isNaN(parsed.getTime())) return date;
	return parsed.toLocaleDateString("en-US", {
		year: "numeric",
		month: "long",
		day: "numeric",
		timeZone: "UTC",
	});
}
