// Small ruleset: typescript-eslint's recommended rules (type-unaware, fast) plus the Solid reactivity checks.
// Run `npm run lint`. Reactivity findings are errors (F22); the rest stay warnings so the gate only blocks real errors.
import js from "@eslint/js";
import globals from "globals";
import solid from "eslint-plugin-solid/configs/typescript";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist", "node_modules", "src-tauri"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    ...solid,
    languageOptions: { ...solid.languageOptions, globals: globals.browser },
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    rules: { "solid/reactivity": "error" },
  },
  {
    // tests build components by hand (createComponent in an array), outside a render scope
    files: ["src/**/*.test.ts"],
    rules: { "solid/reactivity": "off" },
  },
  {
    rules: {
      "@typescript-eslint/no-unused-vars": ["warn", { argsIgnorePattern: "^_", varsIgnorePattern: "^_", caughtErrors: "none" }],
      "@typescript-eslint/no-explicit-any": "warn",
      "@typescript-eslint/no-unused-expressions": ["warn", { allowShortCircuit: true, allowTernary: true }],
    },
  },
);
