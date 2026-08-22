/**
 * The same rule the backend enforces in `skill_name_problem`, repeated here so
 * the form can object while the name is being typed rather than after a failed
 * round trip. A code rather than a sentence, because the sentence is translated.
 */
export type SkillNameProblem = 'empty' | 'tooLong' | 'charset' | 'hyphenEdge';

const MAX_SKILL_NAME_LENGTH = 64;

export function skillNameProblem(name: string): SkillNameProblem | undefined {
  if (name.length === 0) {
    return 'empty';
  }
  if (name.length > MAX_SKILL_NAME_LENGTH) {
    return 'tooLong';
  }
  if (!/^[a-z0-9-]+$/.test(name)) {
    return 'charset';
  }
  if (name.startsWith('-') || name.endsWith('-')) {
    return 'hyphenEdge';
  }
  return undefined;
}
