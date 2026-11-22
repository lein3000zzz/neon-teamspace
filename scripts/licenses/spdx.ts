/**
 * Just enough SPDX to decide whether a package.json license is acceptable:
 * identifiers, `WITH` exceptions, `AND`, `OR` and parentheses. `OR` binds
 * looser than `AND`, as in the SPDX spec.
 */

type Expression =
  { kind: "license"; id: string } | { kind: "and" | "or"; left: Expression; right: Expression };

function tokenize(text: string): string[] {
  return text.replaceAll("(", " ( ").replaceAll(")", " ) ").trim().split(/\s+/).filter(Boolean);
}

/** Throws on anything that isn't a well-formed expression, such as "SEE LICENSE IN ...". */
export function parseSpdx(text: string): Expression {
  const tokens = tokenize(text);
  let position = 0;

  const peek = () => tokens[position];
  const next = () => {
    const token = tokens[position++];
    if (token === undefined) {
      throw new Error(`unexpected end of license expression "${text}"`);
    }
    return token;
  };

  function primary(): Expression {
    const token = next();
    if (token === "(") {
      const inner = or();
      if (next() !== ")") {
        throw new Error(`unbalanced parentheses in "${text}"`);
      }
      return inner;
    }
    if (["AND", "OR", "WITH", ")"].includes(token) || !/^[A-Za-z0-9.+-]+$/.test(token)) {
      throw new Error(`unexpected "${token}" in license expression "${text}"`);
    }
    if (peek() === "WITH") {
      next();
      return { kind: "license", id: `${token} WITH ${next()}` };
    }
    return { kind: "license", id: token };
  }

  function and(): Expression {
    let left = primary();
    while (peek() === "AND") {
      next();
      left = { kind: "and", left, right: primary() };
    }
    return left;
  }

  function or(): Expression {
    let left = and();
    while (peek() === "OR") {
      next();
      left = { kind: "or", left, right: and() };
    }
    return left;
  }

  const expression = or();
  if (position !== tokens.length) {
    throw new Error(`trailing "${tokens.slice(position).join(" ")}" in "${text}"`);
  }
  return expression;
}

/** True when the expression can be satisfied using only allowed licenses. */
export function satisfies(expression: Expression, allowed: ReadonlySet<string>): boolean {
  switch (expression.kind) {
    case "license":
      return allowed.has(expression.id);
    case "and":
      return satisfies(expression.left, allowed) && satisfies(expression.right, allowed);
    case "or":
      return satisfies(expression.left, allowed) || satisfies(expression.right, allowed);
  }
}

/**
 * The license a package.json declares, as one SPDX expression. A list of
 * licenses gives alternatives, so it becomes an OR: the legacy `licenses`
 * array, or an array in `license`, as some old packages have.
 */
export function declaredLicense(manifest: Record<string, unknown>): string | undefined {
  const { license, licenses } = manifest;
  if (typeof license === "string") {
    return license;
  }
  const types = licenseEntries(license, licenses)
    .map((entry: unknown) =>
      typeof entry === "object" && entry !== null && "type" in entry ? entry.type : entry,
    )
    .filter((type): type is string => typeof type === "string");
  return types.length === 0 ? undefined : types.map((type) => `(${type})`).join(" OR ");
}

function licenseEntries(license: unknown, licenses: unknown): unknown[] {
  if (Array.isArray(license)) {
    return license;
  }
  if (Array.isArray(licenses)) {
    return licenses;
  }
  return license === undefined ? [] : [license];
}
