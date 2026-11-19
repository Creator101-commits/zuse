import { Atom } from "effect/unstable/reactivity";
import { describe, expect, it, vi } from "vitest";

import { appAtomRegistry, batchAtomUpdates } from "../../../src/store/registry";

describe("atom store conventions", () => {
	it("batchAtomUpdates coalesces multi-atom writes into one notification", () => {
		const first = Atom.make(0).pipe(Atom.keepAlive);
		const second = Atom.make(0).pipe(Atom.keepAlive);
		const both = Atom.make((get) => get(first) + get(second));
		const listener = vi.fn();
		const unsubscribe = appAtomRegistry.subscribe(both, listener, {
			immediate: true,
		});
		listener.mockClear();

		batchAtomUpdates(() => {
			appAtomRegistry.set(first, 1);
			appAtomRegistry.set(second, 2);
		});

		expect(appAtomRegistry.get(both)).toBe(3);
		expect(listener).toHaveBeenCalledTimes(1);
		unsubscribe();
	});
});
