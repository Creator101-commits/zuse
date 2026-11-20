import { cn } from "@/lib/utils";

/** Neutral letter badge for an agent role, e.g. "B" for Builder, "R" for Reviewer. */
export function AgentBadge({
	mark,
	className,
}: {
	mark: string;
	className?: string;
}) {
	return (
		<span
			aria-hidden
			className={cn(
				"inline-flex shrink-0 items-center justify-center rounded-[4px] bg-foreground/10 font-mono text-[9px] leading-none text-foreground/80",
				className,
			)}
		>
			{mark}
		</span>
	);
}
