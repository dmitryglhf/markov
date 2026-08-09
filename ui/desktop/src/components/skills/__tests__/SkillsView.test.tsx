import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, type RenderOptions, screen, fireEvent, waitFor } from '@testing-library/react';
import type { SourceEntry } from '@aaif/goose-sdk';
import { IntlTestWrapper } from '../../../i18n/test-utils';
import SkillsView from '../SkillsView';

const listSkillSources = vi.fn();
const createSkill = vi.fn();
const deleteSkill = vi.fn();

vi.mock('../../../acp/sources', () => ({
  listSkillSources: (...args: unknown[]) => listSkillSources(...args),
  createSkill: (...args: unknown[]) => createSkill(...args),
  updateSkill: vi.fn(),
  deleteSkill: (...args: unknown[]) => deleteSkill(...args),
}));

vi.mock('../../../utils/workingDir', () => ({
  getInitialWorkingDir: () => '/work/project',
}));

vi.mock('../../../toasts', () => ({
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

const renderWithIntl = (ui: React.ReactElement, options?: RenderOptions) =>
  render(ui, { wrapper: IntlTestWrapper, ...options });

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

describe('SkillsView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('asks for the shadowed copies too, and marks the one nobody uses', async () => {
    listSkillSources.mockResolvedValue([
      skill('review', '/project/.agents/skills/review', { global: false }),
      skill('review', '/home/.agents/skills/review'),
    ]);

    renderWithIntl(<SkillsView />);

    await waitFor(() => expect(screen.getAllByText('review')).toHaveLength(2));
    expect(listSkillSources).toHaveBeenCalledWith('/work/project', true);
    expect(screen.getByText('· shadowed')).toBeInTheDocument();
    expect(screen.getByText('project')).toBeInTheDocument();
    expect(screen.getByText('global')).toBeInTheDocument();
  });

  it('offers no edit or remove for what it does not own', async () => {
    listSkillSources.mockResolvedValue([
      skill('mine', '/home/.agents/skills/mine'),
      skill('from-kit', '/home/.agents/plugins/kit/skills/from-kit', { writable: false }),
      skill('goose-doc-guide', 'builtin://skills/goose-doc-guide', { type: 'builtinSkill' }),
    ]);

    renderWithIntl(<SkillsView />);

    await waitFor(() => expect(screen.getByText('mine')).toBeInTheDocument());
    expect(screen.getByLabelText('Edit mine')).toBeInTheDocument();
    expect(screen.queryByLabelText('Edit from-kit')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Remove from-kit')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Edit goose-doc-guide')).not.toBeInTheDocument();
  });

  it('names the directory it is about to delete before asking', async () => {
    listSkillSources.mockResolvedValue([skill('mine', '/home/.agents/skills/mine')]);

    renderWithIntl(<SkillsView />);

    await waitFor(() => expect(screen.getByText('mine')).toBeInTheDocument());
    fireEvent.click(screen.getByLabelText('Remove mine'));

    expect(screen.getByText('Delete "mine"?')).toBeInTheDocument();
    expect(
      screen.getByText('This removes the whole directory: /home/.agents/skills/mine')
    ).toBeInTheDocument();
  });

  it('creates into the scope that was picked', async () => {
    listSkillSources.mockResolvedValue([]);
    createSkill.mockResolvedValue(skill('fresh', '/work/project/.agents/skills/fresh'));

    renderWithIntl(<SkillsView />);

    await waitFor(() => expect(screen.getByText('No skills installed')).toBeInTheDocument());
    fireEvent.click(screen.getByText('Add Skill'));

    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'fresh' } });
    fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'for testing' } });
    fireEvent.click(screen.getByText('This project only'));
    fireEvent.change(screen.getByLabelText('Instructions'), { target: { value: 'Body' } });
    fireEvent.click(screen.getByText('Save'));

    await waitFor(() =>
      expect(createSkill).toHaveBeenCalledWith('fresh', 'for testing', 'Body', {
        scope: 'projectDir',
        projectDir: '/work/project',
      })
    );
  });
});
