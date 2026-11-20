import { useId } from "react";
import type { ReleaseCover as ReleaseCoverData, Shot } from "@/lib/changelog-arcade";

type Point = readonly [number, number];

// Nine angled frames on a 300×400 board, separated by ~6px slanted gutters.
// Frame 4 sits under the centre badge.
const frames: readonly (readonly Point[])[] = [
	[[0, 0], [92, 0], [86, 118], [0, 124]],
	[[98, 0], [208, 0], [214, 108], [92, 118]],
	[[214, 0], [300, 0], [300, 104], [220, 108]],
	[[0, 130], [86, 124], [80, 262], [0, 268]],
	[[92, 124], [214, 114], [220, 252], [86, 262]],
	[[220, 114], [300, 110], [300, 246], [226, 252]],
	[[0, 274], [80, 268], [88, 400], [0, 400]],
	[[86, 268], [220, 258], [206, 400], [94, 400]],
	[[226, 258], [300, 252], [300, 400], [212, 400]],
];

const PAD = 8;

function bounds(points: readonly Point[]) {
	const xs = points.map((p) => p[0]);
	const ys = points.map((p) => p[1]);
	const x = Math.min(...xs);
	const y = Math.min(...ys);
	return { x, y, width: Math.max(...xs) - x, height: Math.max(...ys) - y };
}

function aspect(focus: string): string {
	// Map a CSS object-position like "30% 20%" to the closest SVG alignment.
	const [fx = "50%", fy = "50%"] = focus.split(" ");
	const pick = (v: string) => {
		const n = Number.parseFloat(v);
		return n < 34 ? "Min" : n > 66 ? "Max" : "Mid";
	};
	return `x${pick(fx)}Y${pick(fy)} slice`;
}

/** Poster-style cover: real product shots cut into angled frames around a wordmark. */
export function CoverArt({
	cover,
	className,
	showBadge = true,
}: {
	cover: ReleaseCoverData;
	className?: string;
	showBadge?: boolean;
}) {
	const uid = useId().replace(/:/g, "");
	const [major, ...build] = cover.version.split("-");
	const label = build.length > 0 ? `${major} · ${build.join("-")}` : (major ?? cover.version);
	return (
		<svg
			viewBox={`0 0 ${300 + PAD * 2} ${400 + PAD * 2}`}
			className={className}
			role="img"
			aria-label={`Zuse ${cover.version} cover`}
		>
			<defs>
				{frames.map((points, i) => (
					<clipPath key={i} id={`${uid}-f${i}`}>
						<polygon points={points.map(([x, y]) => `${x + PAD},${y + PAD}`).join(" ")} />
					</clipPath>
				))}
				<pattern id={`${uid}-scan`} width="3" height="3" patternUnits="userSpaceOnUse">
					<rect width="3" height="1" fill="#000" />
				</pattern>
			</defs>

			<rect width="100%" height="100%" fill="#0b0912" />

			{frames.map((points, i) => {
				const s = cover.shots[i] as Shot | undefined;
				if (!s) return null;
				const b = bounds(points);
				return (
					<g key={i} clipPath={`url(#${uid}-f${i})`}>
						<rect x={b.x + PAD} y={b.y + PAD} width={b.width} height={b.height} fill="#1a1724" />
						<image
							href={s.src}
							x={b.x + PAD}
							y={b.y + PAD}
							width={b.width}
							height={b.height}
							preserveAspectRatio={aspect(s.focus)}
						/>
					</g>
				);
			})}

			{/* Faint CRT scanlines over the artwork. */}
			<rect width="100%" height="100%" fill={`url(#${uid}-scan)`} opacity="0.18" />

			{showBadge && (
				<g transform={`translate(${150 + PAD} ${190 + PAD}) rotate(-4)`}>
					<polygon
						points="-84,-46 80,-52 86,42 -78,48"
						fill="#0b0912"
						stroke="rgb(255 255 255 / 0.22)"
						strokeWidth="1"
					/>
					<text
						x="0"
						y="8"
						textAnchor="middle"
						className="cv-cover-word"
						fill="#fff"
					>
						ZUSE
					</text>
					<text
						x="0"
						y="34"
						textAnchor="middle"
						className="cv-cover-version"
						fill="rgb(255 255 255 / 0.6)"
						style={label.length > 10 ? { fontSize: 8.5, letterSpacing: 1 } : undefined}
					>
						{label}
					</text>
				</g>
			)}
		</svg>
	);
}

/** Shelf card: the cover art plus a quiet label underneath. */
export function ReleaseCover({ cover }: { cover: ReleaseCoverData }) {
	return (
		<div className="cv-cover">
			<CoverArt cover={cover} className="cv-cover-art" />
		</div>
	);
}
