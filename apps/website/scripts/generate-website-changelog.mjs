#!/usr/bin/env node
// Builds content/changelog.json from the repository's root CHANGELOG.md.
// Accepts both "## 0.26.1 - 2026-11-15" and "## [0.26.1] - 2026-11-15" headings.
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const appRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const changelogPath = join(appRoot, "..", "..", "CHANGELOG.md");
const outputPath = join(appRoot, "content", "changelog.json");

export function parseChangelog(source) {
	const releases = [];
	let release = null;
	let section = null;

	for (const line of source.split(/\r?\n/)) {
		const heading = line.match(
			/^## \[?([^\]\s]+)\]?(?:\s*[-–]\s*(\d{4}-\d{2}-\d{2}))?\s*$/,
		);
		if (heading) {
			if (release) releases.push(release);
			release = { version: heading[1], date: heading[2] ?? null, sections: [] };
			section = null;
			continue;
		}

		const sectionHeading = line.match(/^### (.+)$/);
		if (sectionHeading && release) {
			const title = sectionHeading[1].trim();
			section = release.sections.find((s) => s.title === title) ?? null;
			if (!section) {
				section = { title, items: [] };
				release.sections.push(section);
			}
			continue;
		}

		if (line.startsWith("- ") && section) {
			section.items.push(line.slice(2).trim());
		} else if (/^\s{2,}\S/.test(line) && section?.items.length) {
			// Continuation line of a wrapped bullet.
			section.items[section.items.length - 1] += ` ${line.trim()}`;
		}
	}
	if (release) releases.push(release);

	return releases.filter(
		(r) =>
			r.version.toLowerCase() !== "unreleased" &&
			r.sections.some((s) => s.items.length > 0),
	);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
	const releases = parseChangelog(readFileSync(changelogPath, "utf8"));
	writeFileSync(outputPath, `${JSON.stringify(releases, null, "\t")}\n`);
	console.log(`Wrote ${releases.length} changelog releases to ${outputPath}`);
}
