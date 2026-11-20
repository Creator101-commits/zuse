import type { ReactNode } from "react";

const token = /\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)|\*\*([^*]+)\*\*|`([^`]+)`/g;

/** Renders the small Markdown subset used in CHANGELOG.md bullets. */
export function RichText({ text }: { text: string }) {
	const out: ReactNode[] = [];
	let offset = 0;
	for (const match of text.matchAll(token)) {
		const index = match.index ?? 0;
		out.push(text.slice(offset, index));
		if (match[1] && match[2]) {
			out.push(
				<a key={index} href={match[2]} className="underline underline-offset-2">
					{match[1]}
				</a>,
			);
		} else if (match[3]) {
			out.push(<strong key={index}>{match[3]}</strong>);
		} else if (match[4]) {
			out.push(<code key={index}>{match[4]}</code>);
		}
		offset = index + match[0].length;
	}
	out.push(text.slice(offset));
	return <>{out}</>;
}
