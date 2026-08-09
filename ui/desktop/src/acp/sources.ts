import type { SourceEntry, SourceScope, SourceType } from '@aaif/goose-sdk';
import { getAcpClient } from './acpConnection';

const SKILL_SOURCE_TYPES: SourceType[] = ['skill', 'builtinSkill'];
const inFlightSkillSourceLoads = new Map<string, Promise<SourceEntry[]>>();

export async function listSkillSources(
  projectDir: string,
  includeShadowed = false
): Promise<SourceEntry[]> {
  const key = `${includeShadowed ? 'all' : 'used'}:${projectDir}`;
  const inFlightLoad = inFlightSkillSourceLoads.get(key);
  if (inFlightLoad) {
    return inFlightLoad;
  }

  const load = loadSkillSources(projectDir, includeShadowed);
  inFlightSkillSourceLoads.set(key, load);

  try {
    return await load;
  } finally {
    if (inFlightSkillSourceLoads.get(key) === load) {
      inFlightSkillSourceLoads.delete(key);
    }
  }
}

async function loadSkillSources(
  projectDir: string,
  includeShadowed: boolean
): Promise<SourceEntry[]> {
  const client = await getAcpClient();
  const responses = await Promise.all(
    SKILL_SOURCE_TYPES.map((type) =>
      client.goose.sourcesList_unstable({
        type,
        projectDir,
        includeShadowed,
      })
    )
  );

  return responses
    .flatMap((response) => response.sources)
    .sort(
      (a, b) =>
        a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }) ||
        a.path.localeCompare(b.path)
    );
}

export async function createSkill(
  name: string,
  description: string,
  content: string,
  target: SourceScope
): Promise<SourceEntry> {
  const client = await getAcpClient();
  const response = await client.goose.sourcesCreate_unstable({
    type: 'skill',
    name,
    description,
    content,
    target,
  });
  return response.source;
}

// `properties` is deliberately absent: omitting it tells the backend to keep the
// metadata already in the frontmatter, which this editor does not model.
export async function updateSkill(
  path: string,
  name: string,
  description: string,
  content: string
): Promise<SourceEntry> {
  const client = await getAcpClient();
  const response = await client.goose.sourcesUpdate_unstable({
    type: 'skill',
    path,
    name,
    description,
    content,
  });
  return response.source;
}

export async function deleteSkill(path: string): Promise<void> {
  const client = await getAcpClient();
  await client.goose.sourcesDelete_unstable({ type: 'skill', path });
}
