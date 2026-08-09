import type { SourceEntry } from '@aaif/goose-sdk';

/// One copy of a skill on disk. Several copies can share a name, and only the
/// first one discovery meets is ever used, so each copy carries the path of the
/// one that beat it.
export type FoundSkill = {
  entry: SourceEntry;
  shadowedBy?: string;
};

export type SkillOrigin = 'builtin' | 'plugin' | 'global' | 'project';

/**
 * Pair every copy with the one that answers to its name. Mirrors the discovery
 * order the agent itself uses: the first copy of a name wins.
 */
export function withShadows(entries: SourceEntry[]): FoundSkill[] {
  const winners = new Map<string, string>();

  return entries.map((entry) => {
    const winner = winners.get(entry.name) ?? entry.path;
    winners.set(entry.name, winner);
    return winner === entry.path ? { entry } : { entry, shadowedBy: winner };
  });
}

export function skillOrigin(entry: SourceEntry): SkillOrigin {
  if (entry.type === 'builtinSkill') {
    return 'builtin';
  }
  // A skill a plugin brought keeps its source of truth in that plugin's
  // repository, so calling it "global" would invite editing something the next
  // update overwrites.
  if (entry.writable !== true) {
    return 'plugin';
  }
  return entry.global ? 'global' : 'project';
}

export function isEditable(entry: SourceEntry): boolean {
  return entry.type === 'skill' && entry.writable === true;
}

/** Other copies of this name, none of which the agent will ever load. */
export function alsoAt(skill: FoundSkill, found: FoundSkill[]): string[] {
  if (skill.shadowedBy) {
    return [];
  }
  return found
    .filter(
      (other) => other.entry.name === skill.entry.name && other.entry.path !== skill.entry.path
    )
    .map((other) => other.entry.path);
}
