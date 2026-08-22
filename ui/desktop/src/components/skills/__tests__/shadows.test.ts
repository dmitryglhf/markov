import { describe, expect, it } from 'vitest';
import type { SourceEntry } from '@aaif/goose-sdk';
import { alsoAt, isEditable, skillOrigin, withShadows } from '../shadows';

function skill(name: string, path: string, overrides: Partial<SourceEntry> = {}): SourceEntry {
  return {
    type: 'skill',
    name,
    description: `what ${name} is for`,
    content: '',
    path,
    global: true,
    writable: true,
    ...overrides,
  };
}

describe('withShadows', () => {
  it('gives every copy the path of the one that answers to its name', () => {
    const found = withShadows([
      skill('review', '/project/.agents/skills/review', { global: false }),
      skill('review', '/home/.claude/skills/review'),
      skill('deploy', '/home/.agents/skills/deploy'),
    ]);

    expect(found[0].shadowedBy).toBeUndefined();
    expect(found[1].shadowedBy).toBe('/project/.agents/skills/review');
    expect(found[2].shadowedBy).toBeUndefined();
  });

  it('lists the unused copies against the one in use', () => {
    const found = withShadows([skill('review', '/a/review'), skill('review', '/b/review')]);

    expect(alsoAt(found[0], found)).toEqual(['/b/review']);
    expect(alsoAt(found[1], found)).toEqual([]);
  });
});

describe('skillOrigin', () => {
  it('separates what we own from what only passes through', () => {
    expect(skillOrigin(skill('a', '/home/.agents/skills/a'))).toBe('global');
    expect(skillOrigin(skill('a', '/project/.agents/skills/a', { global: false }))).toBe('project');
    expect(skillOrigin(skill('a', '/home/.agents/plugins/kit/skills/a', { writable: false }))).toBe(
      'plugin'
    );
    expect(skillOrigin(skill('a', 'builtin://skills/a', { type: 'builtinSkill' }))).toBe('builtin');
  });
});

describe('isEditable', () => {
  /// Editing a plugin's copy is undone by its next update, and removing it
  /// deletes a directory inside somebody else's checkout.
  it('offers only what we own', () => {
    expect(isEditable(skill('a', '/home/.agents/skills/a'))).toBe(true);
    expect(isEditable(skill('a', '/plugins/kit/skills/a', { writable: false }))).toBe(false);
    expect(isEditable(skill('a', 'builtin://skills/a', { type: 'builtinSkill' }))).toBe(false);
  });

  it('treats a missing flag as not ours', () => {
    expect(isEditable(skill('a', '/home/.agents/skills/a', { writable: undefined }))).toBe(false);
  });
});
