"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { type ChangeRelease, formatReleaseDate, releaseCover } from "@/lib/changelog-arcade";
import { ReleaseCollage } from "./collage";
import { ReleaseCover } from "./cover";

import "@fontsource/anton/latin-400.css";
import "./arcade.css";

const hashFor = (version: string) => `#v${version}`;

export function ArcadeChangelog({ releases }: { releases: ChangeRelease[] }) {
	const covers = useMemo(() => releases.map(releaseCover), [releases]);
	const [active, setActive] = useState(0);
	const [open, setOpen] = useState(false);
	const total = releases.length;
	const touchX = useRef<number | null>(null);

	const select = useCallback(
		(index: number) => setActive(Math.max(0, Math.min(total - 1, index))),
		[total],
	);

	// Deep links: /changelog#v0.26.1 opens that release.
	useEffect(() => {
		const openFromHash = () => {
			const index = releases.findIndex((r) => hashFor(r.version) === window.location.hash);
			if (index >= 0) {
				setActive(index);
				setOpen(true);
			}
		};
		openFromHash();
		window.addEventListener("hashchange", openFromHash);
		return () => window.removeEventListener("hashchange", openFromHash);
	}, [releases]);

	useEffect(() => {
		const release = releases[active];
		if (!release) return;
		const url = open ? hashFor(release.version) : window.location.pathname;
		window.history.replaceState(null, "", url);
	}, [active, open, releases]);

	useEffect(() => {
		document.documentElement.style.overflow = open ? "hidden" : "";
		return () => {
			document.documentElement.style.overflow = "";
		};
	}, [open]);

	useEffect(() => {
		const onKey = (event: KeyboardEvent) => {
			const target = event.target instanceof HTMLElement ? event.target : null;
			if (target?.closest("input, textarea, select, [contenteditable]")) return;
			if (event.key === "Escape" && open) {
				setOpen(false);
			} else if (event.key === "ArrowRight") {
				event.preventDefault();
				select(active + 1);
			} else if (event.key === "ArrowLeft") {
				event.preventDefault();
				select(active - 1);
			} else if (event.key === "Enter" && !open && !target?.closest("a, button")) {
				// Stop the same key press from activating the close button that
				// receives focus once the collage opens.
				event.preventDefault();
				setOpen(true);
			}
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [active, open, select]);

	const cover = covers[active];
	const release = releases[active];
	if (!cover || !release) return null;

	return (
		<section className="cv-stage" aria-label="Releases">
			<div className="cv-info">
				<p className="cv-eyebrow">
					Rel. {String(total - active).padStart(2, "0")}
					{cover.preview && " — Preview"}
				</p>
				<h2 className="cv-version">
					{cover.version.split("-")[0]}
					<i className="cv-caret" aria-hidden />
				</h2>
				{cover.version.includes("-") && (
					<p className="cv-build">{cover.version.split("-").slice(1).join("-")}</p>
				)}
				<p className="cv-title">{cover.title}</p>
				<p className="cv-meta">
					{formatReleaseDate(cover.date)}
					<span>·</span>
					{cover.changes} changes
				</p>
				<button type="button" className="cv-open" onClick={() => setOpen(true)}>
					View release <kbd aria-hidden>↵</kbd>
				</button>
			</div>

			<div
				className="cv-shelf"
				onTouchStart={(event) => {
					touchX.current = event.touches[0]?.clientX ?? null;
				}}
				onTouchEnd={(event) => {
					const start = touchX.current;
					const end = event.changedTouches[0]?.clientX;
					touchX.current = null;
					if (start == null || end == null || Math.abs(end - start) < 40) return;
					select(active + (end < start ? 1 : -1));
				}}
			>
				{covers.map((item, index) => {
					const d = index - active;
					const distance = Math.abs(d);
					const hidden = distance > 3 || d < -1;
					return (
						<button
							key={item.version}
							type="button"
							className="cv-shelf-item"
							data-active={d === 0 || undefined}
							aria-label={`${item.version}: ${item.title}`}
							aria-current={d === 0 || undefined}
							tabIndex={hidden ? -1 : 0}
							onClick={() => (d === 0 ? setOpen(true) : select(index))}
							style={{
								transform: `translate(-50%, -50%) translate3d(${d * 175}px, ${d * -34}px, ${distance * -220}px) rotateY(-22deg)`,
								zIndex: 100 - distance,
								opacity: hidden ? 0 : d === 0 ? 1 : d < 0 ? 0.25 : Math.max(0.2, 0.6 - distance * 0.15),
								pointerEvents: hidden ? "none" : undefined,
							}}
						>
							<ReleaseCover cover={item} />
						</button>
					);
				})}
			</div>

			<div className="cv-dots" aria-hidden>
				<span className="cv-dots-count">
					{String(active + 1).padStart(2, "0")} / {String(total).padStart(2, "0")}
				</span>
				{covers.map((item, index) => (
					<span key={item.version} className="cv-dot" data-active={index === active || undefined} />
				))}
			</div>

			{open && (
				<ReleaseCollage
					release={release}
					cover={cover}
					number={active + 1}
					total={total}
					onClose={() => setOpen(false)}
					onStep={(delta) => select(active + delta)}
				/>
			)}
		</section>
	);
}
