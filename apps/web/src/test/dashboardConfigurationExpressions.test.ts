import { describe, expect, it } from 'vitest';
import {
  parseSafeMathExpression,
  resolveSafeExpression,
  resolveSafeNumberPath,
} from '../../../../packages/dashboards/src/charts/expressions';

describe('configuration arithmetic contracts', () => {
  it.each([
    ['2 + 3 * 4', 14], ['(2 + 3) * 4', 20], ['12 / 3 / 2', 2], ['12 - 3 - 2', 7],
    ['--2', 2], ['-+-2', 2], ['1++2', 3], ['2 * -3', -6], ['-(2 + 3)', -5],
    ['.5 + 2. + 2E-1', 2.7], ['1e+2 / 1E2', 1], ['\t2\n+\u00a03', 5],
    ['0002', 2], ['-0', -0], ['0 / -2', -0], ['1e-9999', 0],
    ['1 / (1e308 * 1e308)', 0], ['1 / (1e308 * 1e308 - 1e308)', 0],
  ])('evaluates %s', (formula, expected) => {
    expect(parseSafeMathExpression(formula)).toBe(expected);
  });

  it.each([
    '', ' ', '.', '1e', '1e+', '1 2', '2(3)', '(2)3', '()', '(1', '1)', '1+',
    '*2', '2**3', '2//3', '1,2', '1_000', '0xff', 'NaN', 'Infinity', '1e9999',
    '1/0', '1/-0', '0*(1/0)', '1e308*1e308', '0*(1e308*1e308)',
    'Math.max(1,2)', 'globalThis.value', 'alert(1)', '[x]', '1;2', '1/*x*/+2',
  ])('rejects malformed or nonfinite arithmetic: %s', (formula) => {
    expect(parseSafeMathExpression(formula)).toBeNull();
  });

  it('accepts 512 characters and rejects 513, including signs and nesting', () => {
    expect(parseSafeMathExpression('1'.padEnd(512))).toBe(1);
    expect(parseSafeMathExpression('1'.padEnd(513))).toBeNull();
    expect(parseSafeMathExpression(`${'+'.repeat(511)}1`)).toBe(1);
    expect(parseSafeMathExpression(`${'('.repeat(255)}1${')'.repeat(255)}`)).toBe(1);
    expect(resolveSafeExpression('[n]'.padEnd(512), { n: 1 })).toBe(1);
    expect(resolveSafeExpression('[n]'.padEnd(513), { n: 1 })).toBeNull();
  });

  it('checks both original and substituted formula lengths', () => {
    const formula = Array.from({ length: 40 }, () => '[n]').join('+');
    expect(formula.length).toBeLessThan(512);
    expect(resolveSafeExpression(formula, { n: 123456789012345 })).toBeNull();
  });

  it('retains textual substitution including adjacency, signs and negative zero', () => {
    expect(resolveSafeExpression('[a][b]', { a: 2, b: 3 })).toBe(23);
    expect(resolveSafeExpression('[a][b]', { a: 2, b: -3 })).toBe(-1);
    expect(resolveSafeExpression('[a].[b]', { a: 2, b: 3 })).toBe(2.3);
    expect(resolveSafeExpression('[a]', { a: -0 })).toBe(0);
    expect(resolveSafeExpression('[ a ] * [nested.b]', { a: -2, nested: { b: 3 } })).toBe(-6);
    expect(resolveSafeExpression('0 * [missing]', {})).toBeNull();
  });

  it.each([null, undefined, '2', true, NaN, Infinity, -Infinity, {}, []])('rejects nonfinite or nonnumeric paths: %j', (value) => {
    expect(resolveSafeNumberPath({ value }, 'value')).toBeNull();
    expect(resolveSafeExpression('[value]', { value })).toBeNull();
  });

  it('preserves path traversal, including arrays, inherited numeric members and empty segments', () => {
    const inherited = Object.create({ inherited: 3 }) as Record<string, unknown>;
    inherited.nested = { '': { value: 4 } };
    inherited.items = [5];
    inherited[' padded '] = 6;
    expect(resolveSafeNumberPath(inherited, 'inherited')).toBe(3);
    expect(resolveSafeNumberPath(inherited, 'nested..value')).toBe(4);
    expect(resolveSafeNumberPath(inherited, 'items.0')).toBe(5);
    expect(resolveSafeNumberPath(inherited, 'items.length')).toBe(1);
    expect(resolveSafeNumberPath(inherited, ' padded ')).toBe(6);
    expect(resolveSafeNumberPath(inherited, '')).toBeNull();
    expect(resolveSafeNumberPath({ value: 'abc' }, 'value.length')).toBeNull();
    expect(resolveSafeNumberPath({ value: () => 2 }, 'value.length')).toBeNull();
    expect(resolveSafeExpression('[ padded ]', inherited)).toBeNull();
  });

  it('does not mutate source values', () => {
    const values = Object.freeze({ nested: Object.freeze({ value: 4 }) });
    expect(resolveSafeExpression('[nested.value] / 2', values)).toBe(2);
    expect(values).toEqual({ nested: { value: 4 } });
  });
});
