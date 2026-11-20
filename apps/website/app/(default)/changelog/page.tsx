import { ArcadeChangelog } from "@/components/changelog-arcade/arcade-changelog";
import { ChangelogText } from "@/components/changelog-text";
import { Container } from "@/components/container";
import { PageMasthead } from "@/components/page-masthead";
import changelogData from "@/content/changelog.json";
import { type ChangeRelease, formatReleaseDate } from "@/lib/changelog-arcade";
import { getSEO } from "@/lib/seo";

export const metadata = getSEO({
	title: "Change Log",
	description: "All notable Zuse product changes, grouped by release.",
	path: "/changelog",
});

export default function ChangelogPage() {
	const releases = changelogData as ChangeRelease[];

	return (
		<main className="w-full">
			<PageMasthead
				eyebrow="Release notes"
				title="Every release, framed."
				description="Pick a release to see everything that landed in it."
			/>

			<ArcadeChangelog releases={releases} />

			<Container className="flex flex-col pb-24">
				<h2 className="text-heading mt-16 font-mono text-xs font-semibold tracking-wide uppercase">
					Full patch notes
				</h2>
				<div className="grid gap-8">
					{releases.map((release) => (
						<article
							key={release.version}
							id={`notes-${release.version}`}
							className="border-b border-dotted border-border py-12"
						>
							<div className="flex flex-col gap-8 md:grid md:grid-cols-[160px_1fr]">
								<div>
									<h3 className="text-primary font-mono text-lg font-semibold">{release.version}</h3>
									<p className="text-muted-foreground mt-1 font-mono text-xs">
										{formatReleaseDate(release.date)}
									</p>
								</div>
								<div className="grid gap-8">
									{release.sections.map((section) => (
										<section key={`${release.version}-${section.title}`}>
											<h4 className="text-heading font-mono text-xs font-semibold tracking-wide uppercase">
												{section.title}
											</h4>
											<ul className="mt-4 grid gap-3">
												{section.items.map((item) => (
													<li key={item} className="text-muted-foreground flex gap-3 text-sm leading-6">
														<span className="bg-primary mt-2 size-1.5 shrink-0 rounded-full" />
														<ChangelogText text={item} />
													</li>
												))}
											</ul>
										</section>
									))}
								</div>
							</div>
						</article>
					))}
				</div>
			</Container>
		</main>
	);
}
