import { describe, expect, it } from 'vitest';
import { skillNameProblem } from '../skillName';

/// Mirrors `skill_name_problem` in crates/goose/src/skills/mod.rs; the cases are
/// the ones its own test covers, so the two rules cannot drift apart quietly.
describe('skillNameProblem', () => {
  it('accepts what the backend accepts', () => {
    expect(skillNameProblem('my-skill')).toBeUndefined();
    expect(skillNameProblem('abc123')).toBeUndefined();
    expect(skillNameProblem('double--hyphen')).toBeUndefined();
    expect(skillNameProblem('a'.repeat(64))).toBeUndefined();
  });

  it('names the reason it refuses', () => {
    expect(skillNameProblem('')).toBe('empty');
    expect(skillNameProblem('a'.repeat(65))).toBe('tooLong');
    expect(skillNameProblem('CAPS')).toBe('charset');
    expect(skillNameProblem('../escape')).toBe('charset');
    expect(skillNameProblem('-leading')).toBe('hyphenEdge');
    expect(skillNameProblem('trailing-')).toBe('hyphenEdge');
  });
});
