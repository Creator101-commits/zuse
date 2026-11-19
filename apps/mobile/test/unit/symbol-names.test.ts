import { describe, expect, test } from "vitest";
import { platformSymbolName } from "../../src/lib/symbol-names";

describe("platform symbol names", () => {
	test("keeps an unmapped symbol visible on other platforms", () => {
		expect(platformSymbolName("star.fill")).toEqual({
			ios: "star.fill",
			android: "help",
			web: "help",
		});
	});
});
