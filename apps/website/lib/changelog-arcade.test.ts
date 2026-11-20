import { describe, expect, it } from "vitest";
import {
	collagePanels,
	formatReleaseDate,
	headline,
	plainText,
	releaseCover,
	sectionTag,
	shotFor,
} from "./changelog-arcade";

const release = {
	version: "0.26.1",
	date: "2026-11-15",
	sections: [
		{
			title: "Added",
			items: [
				"Choose Stable or Preview updates in General settings. Your selection is saved.",
				"First-run setup for agents, sign-in, MCP servers, and importing chats.",
			],
		},
		{ title: "Fixed", items: ["Correct browser page sizing on high-DPI Linux desktops."] },
	],
};

describe("changelog collage", () => {
	it("strips inline markdown", () => {
		expect(plainText("Show **Provider commands** via `zuse app` and [docs](https://x.y)")).toBe(
			"Show Provider commands via zuse app and docs",
		);
	});

	it("builds short headlines from the first clause", () => {
		expect(headline("Choose Stable or Preview updates in General settings. Your selection is saved.")).toBe(
			"Stable or Preview updates",
		);
		expect(headline("Mermaid diagrams in pull request descriptions and comments.")).toBe(
			"Mermaid diagrams in pull request",
		);
		expect(headline("Search from the browser address bar, see previews")).toBe("Search from the browser address");
	});

	it("tags sections", () => {
		expect(sectionTag("Added")).toBe("New");
		expect(sectionTag("Changed")).toBe("Improved");
		expect(sectionTag("Fixed")).toBe("Fixed");
		expect(sectionTag("Release scope")).toBe("Release scope");
	});

	it("lays out one deterministic panel per item", () => {
		const panels = collagePanels(release);
		expect(panels).toHaveLength(3);
		expect(panels[0]?.large).toBe(true);
		expect(panels[2]?.kind).toBe("fixed");
		expect(collagePanels(release)).toEqual(panels);
	});

	it("summarises a release as a cover", () => {
		const cover = releaseCover(release);
		expect(cover.changes).toBe(3);
		expect(cover.shots).toHaveLength(9);
		expect(new Set(cover.shots.map((s) => s.id)).size).toBe(9);
		expect(cover.preview).toBe(false);
		expect(releaseCover({ ...release, version: "0.26.0-preview.5" }).preview).toBe(true);
	});

	it("matches screenshots to changes without repeating", () => {
		expect(shotFor("Improved pull request layout and review controls").id).toBe("checks");
		expect(shotFor("Link a chat to a Slack thread").id).toBe("connect");
		expect(shotFor("Link a chat to a Slack thread", new Set(["connect"])).id).toBe("conversation");
		const ids = collagePanels(release).map((p) => p.shot.id);
		expect(new Set(ids).size).toBe(ids.length);
	});

	it("formats dates in UTC", () => {
		expect(formatReleaseDate("2026-11-15")).toBe("November 15, 2026");
		expect(formatReleaseDate(null)).toBe("Unreleased");
	});
});
