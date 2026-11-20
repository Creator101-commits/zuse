"use client";

import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import {
	type ChangeRelease,
	collagePanels,
	formatReleaseDate,
	type ReleaseCover,
	sectionTag,
} from "@/lib/changelog-arcade";
import { CoverArt } from "./cover";
import { RichText } from "./rich-text";

/** Full-screen collage: one screenshot panel per change in the release. */
export function ReleaseCollage({
	release,
	cover,
	number,
	total,
	onClose,
	onStep,
}: {
	release: ChangeRelease;
	cover: ReleaseCover;
	number: number;
	total: number;
	onClose: () => void;
	onStep: (delta: number) => void;
}) {
	const panels = collagePanels(release);
	const dialogRef = useRef<HTMLDivElement>(null);
	const closeRef = useRef<HTMLButtonElement>(null);

	useEffect(() => {
		closeRef.current?.focus();
		dialogRef.current?.scrollTo({ top: 0 });
	}, [release.version]);

	return createPortal(
		<div
			ref={dialogRef}
			className="cv-overlay"
			role="dialog"
			aria-modal="true"
			aria-label={`Release ${release.version}`}
		>
			<div className="cv-poster">
				<header className="cv-poster-head">
					<CoverArt cover={cover} className="cv-poster-cover" />
					<div className="cv-poster-info">
						<p className="cv-eyebrow">
							{formatReleaseDate(release.date)} · {cover.changes} changes
						</p>
						<h2 className="cv-poster-title">
							Zuse <span>{release.version}</span>
							<i className="cv-caret" aria-hidden />
						</h2>
						<p className="cv-poster-lead">{cover.title}</p>
						<nav className="cv-poster-nav">
							<button type="button" onClick={() => onStep(-1)} disabled={number <= 1}>
								← Newer
							</button>
							<span>
								{String(number).padStart(2, "0")} / {String(total).padStart(2, "0")}
							</span>
							<button type="button" onClick={() => onStep(1)} disabled={number >= total}>
								Older →
							</button>
							<button ref={closeRef} type="button" onClick={onClose} className="cv-close">
								Close <kbd>esc</kbd>
							</button>
						</nav>
					</div>
				</header>

				<div className="cv-grid">
					{panels.map((panel, index) => (
						<section
							key={panel.id}
							className={`cv-panel${panel.large ? " cv-panel-large" : ""}`}
							style={{ gridColumn: `span ${panel.shape.cols}` }}
						>
							<div className="cv-panel-screen">
								{/* eslint-disable-next-line @next/next/no-img-element -- decorative crop of a static screenshot */}
								<img
									src={panel.shot.src}
									alt=""
									loading="lazy"
									style={{ objectPosition: panel.shot.focus }}
								/>
							</div>
							<div className="cv-panel-copy">
								<p className="cv-panel-meta">
									<span>{String(index + 1).padStart(2, "0")}</span>
									<span>{sectionTag(panel.section)}</span>
								</p>
								<h3>{panel.headline}</h3>
								<p className="cv-panel-text">
									<RichText text={panel.text} />
								</p>
							</div>
						</section>
					))}
				</div>
			</div>
		</div>,
		document.body,
	);
}
