// Finds dictionary reads (S, P, W, B, R, WS, L, U ... built with `dict`) that run once at module load and end in a text
// or a list: they would keep the language of that moment. Run: node scripts/check-i18n-static.mjs (exit 1 when any is found).
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "app");
const ts = createRequire(path.join(root, "package.json"))("typescript");
const cfg = ts.getParsedCommandLineOfConfigFile(
  path.join(root, "tsconfig.json"),
  {},
  { ...ts.sys, onUnRecoverableConfigFileDiagnostic: () => {} },
);
const program = ts.createProgram(cfg.fileNames, cfg.options);
const checker = program.getTypeChecker();

/** the exported `X = dict(...)` symbols */
const dictSymbols = new Set();
for (const sf of program.getSourceFiles()) {
  if (sf.isDeclarationFile || /\.en\.ts$/.test(sf.fileName)) continue;
  ts.forEachChild(sf, (n) => {
    if (!ts.isVariableStatement(n)) return;
    for (const d of n.declarationList.declarations)
      if (d.initializer && ts.isCallExpression(d.initializer) && d.initializer.expression.getText() === "dict")
        dictSymbols.add(checker.getSymbolAtLocation(d.name));
  });
}

/** does this expression derive from a dictionary (directly, or through an alias of one of its nodes)? */
const rootsInDict = (e, seen = new Set()) => {
  while (
    ts.isPropertyAccessExpression(e) ||
    ts.isElementAccessExpression(e) ||
    ts.isNonNullExpression(e) ||
    ts.isParenthesizedExpression(e)
  )
    e = e.expression;
  if (!ts.isIdentifier(e)) return false;
  let sym = checker.getSymbolAtLocation(e);
  if (sym && sym.flags & ts.SymbolFlags.Alias) sym = checker.getAliasedSymbol(sym);
  if (!sym) return false;
  if (dictSymbols.has(sym)) return true;
  if (seen.has(sym)) return false;
  seen.add(sym);
  const d = sym.valueDeclaration;
  return !!d && ts.isVariableDeclaration(d) && !!d.initializer && rootsInDict(d.initializer, seen);
};

const insideFunction = (n) => {
  for (let p = n.parent; p; p = p.parent) if (ts.isFunctionLike(p)) return true;
  return false;
};

let bad = 0;
for (const sf of program.getSourceFiles()) {
  if (
    sf.isDeclarationFile ||
    sf.fileName.includes("node_modules") ||
    /\.test\.tsx?$|[\\/]strings[^\\/]*\.ts$|[\\/]i18n\.ts$/.test(sf.fileName)
  )
    continue;
  const visit = (n) => {
    const chain = ts.isPropertyAccessExpression(n) || ts.isElementAccessExpression(n);
    // only the outermost access of a chain, and only when it runs at load
    if (
      chain &&
      !(
        (ts.isPropertyAccessExpression(n.parent) || ts.isElementAccessExpression(n.parent)) &&
        n.parent.expression === n
      ) &&
      !insideFunction(n) &&
      rootsInDict(n)
    ) {
      const t = checker.getTypeAtLocation(n);
      const text = !!(t.flags & ts.TypeFlags.StringLike) || checker.isArrayType(t) || checker.isTupleType(t);
      // an alias of a node (an object) or of a function is fine: it is read or called later
      if (text) {
        const { line } = sf.getLineAndCharacterOfPosition(n.getStart());
        console.log(`${path.relative(root, sf.fileName)}:${line + 1}  ${n.getText().slice(0, 90)}`);
        bad++;
      }
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
}
console.log(
  bad
    ? `${bad} module-level dictionary read(s) would not follow the language`
    : "no module-level dictionary reads of texts or lists",
);
process.exit(bad ? 1 : 0);
