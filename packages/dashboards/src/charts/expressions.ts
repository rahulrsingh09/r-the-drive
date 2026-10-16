/**
 * Bounded arithmetic only: references become text, tokens become postfix
 * instructions, and a numeric stack evaluates them without JavaScript execution.
 */

export type ExpressionValues = Record<string, unknown>;

const MAX_EXPRESSION_LENGTH = 512;
type BinaryOperator = '+' | '-' | '*' | '/';
type Operator = BinaryOperator | 'positive' | 'negative';
type Token = number | BinaryOperator | '(' | ')';
type Instruction = number | Operator;

export function resolveSafeExpression(formula: string, values: ExpressionValues): number | null {
  if (!hasValidLength(formula)) return null;
  const bound = bindReferences(formula, values);
  return bound === null ? null : parseSafeMathExpression(bound);
}

export function resolveSafeNumberPath(values: ExpressionValues, path: string): number | null {
  if (!path) return null;
  let current: unknown = values;
  for (const part of path.split('.')) {
    if (!current || typeof current !== 'object') return null;
    current = (current as Record<string, unknown>)[part];
  }
  return typeof current === 'number' && Number.isFinite(current) ? current : null;
}

export function parseSafeMathExpression(expression: string): number | null {
  if (!hasValidLength(expression)) return null;
  const tokens = tokenize(expression);
  if (tokens === null) return null;
  const instructions = toPostfix(tokens);
  return instructions === null ? null : evaluate(instructions);
}

function hasValidLength(expression: string): boolean {
  return expression.length > 0 && expression.length <= MAX_EXPRESSION_LENGTH;
}

function bindReferences(formula: string, values: ExpressionValues): string | null {
  const fragments: string[] = [];
  let offset = 0;
  let missing = false;
  for (const match of formula.matchAll(/\[([^\]]+)\]/g)) {
    const value = resolveSafeNumberPath(values, match[1]!.trim());
    fragments.push(formula.slice(offset, match.index), String(value ?? 0));
    offset = match.index + match[0].length;
    missing ||= value === null;
  }
  fragments.push(formula.slice(offset));
  // Saved formulas rely on textual adjacency and String(-0), not numeric tokens.
  return missing ? null : fragments.join('');
}

function tokenize(expression: string): Token[] | null {
  const tokens: Token[] = [];
  const lexeme = /\s+|(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?|[()+*/-]/iy;
  let offset = 0;
  while (offset < expression.length) {
    lexeme.lastIndex = offset;
    const match = lexeme.exec(expression);
    if (!match) return null;
    const text = match[0];
    offset = lexeme.lastIndex;
    if (/^\s/.test(text)) continue;
    if (/^[\d.]/.test(text)) {
      const value = Number(text);
      if (!Number.isFinite(value)) return null;
      tokens.push(value);
    } else {
      tokens.push(text as Exclude<Token, number>);
    }
  }
  return tokens;
}

function precedence(operator: Operator): number {
  switch (operator) {
    case 'positive':
    case 'negative': return 3;
    case '*':
    case '/': return 2;
    case '+':
    case '-': return 1;
  }
}

function toPostfix(tokens: Token[]): Instruction[] | null {
  const output: Instruction[] = [];
  const operators: Array<Operator | '('> = [];
  let needsOperand = true;
  for (const token of tokens) {
    if (typeof token === 'number') {
      if (!needsOperand) return null;
      output.push(token);
      needsOperand = false;
    } else if (token === '(') {
      if (!needsOperand) return null;
      operators.push(token);
    } else if (token === ')') {
      if (needsOperand) return null;
      let operator = operators.pop();
      while (operator !== undefined && operator !== '(') {
        output.push(operator);
        operator = operators.pop();
      }
      if (operator !== '(') return null;
    } else if (needsOperand) {
      if (token !== '+' && token !== '-') return null;
      operators.push(token === '+' ? 'positive' : 'negative');
    } else {
      let previous = operators.at(-1);
      while (previous !== undefined && previous !== '(' && precedence(previous) >= precedence(token)) {
        output.push(operators.pop() as Operator);
        previous = operators.at(-1);
      }
      operators.push(token);
      needsOperand = true;
    }
  }
  if (needsOperand) return null;
  while (operators.length > 0) {
    const operator = operators.pop()!;
    if (operator === '(') return null;
    output.push(operator);
  }
  return output;
}

function evaluate(instructions: Instruction[]): number | null {
  const values: number[] = [];
  for (const instruction of instructions) {
    if (typeof instruction === 'number') {
      values.push(instruction);
      continue;
    }
    const right = values.pop();
    if (right === undefined) return null;
    if (instruction === 'positive' || instruction === 'negative') {
      values.push(instruction === 'negative' ? -right : right);
      continue;
    }
    const left = values.pop();
    if (left === undefined || (instruction === '/' && right === 0)) return null;
    switch (instruction) {
      case '+': values.push(left + right); break;
      case '-': values.push(left - right); break;
      case '*': values.push(left * right); break;
      case '/': values.push(left / right); break;
    }
  }
  const result = values[0];
  // Intermediate overflow can still produce a finite final result, such as 1 / Infinity.
  return values.length === 1 && result !== undefined && Number.isFinite(result) ? result : null;
}
