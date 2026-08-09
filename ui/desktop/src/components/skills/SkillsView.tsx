import { useState, useEffect, useMemo, useCallback } from 'react';
import { Zap, AlertCircle, Plus, Pencil, Trash2 } from 'lucide-react';
import type { SourceEntry } from '@aaif/goose-sdk';
import { ScrollArea } from '../ui/scroll-area';
import { Card } from '../ui/card';
import { Button } from '../ui/button';
import { Skeleton } from '../ui/skeleton';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { errorMessage } from '../../utils/conversionUtils';
import { getInitialWorkingDir } from '../../utils/workingDir';
import { defineMessages, useIntl } from '../../i18n';
import { SearchView } from '../conversation/SearchView';
import { getSearchShortcutText } from '../../utils/keyboardShortcuts';
import { toastError, toastSuccess } from '../../toasts';
import { deleteSkill, listSkillSources } from '../../acp/sources';
import { isEditable, skillOrigin, withShadows, type FoundSkill } from './shadows';
import SkillDetails from './SkillDetails';
import SkillFormModal from './SkillFormModal';

const i18n = defineMessages({
  errorLoadingSkills: {
    id: 'skillsView.errorLoadingSkills',
    defaultMessage: 'Error Loading Skills',
  },
  tryAgain: {
    id: 'skillsView.tryAgain',
    defaultMessage: 'Try Again',
  },
  noSkillsInstalled: {
    id: 'skillsView.noSkillsInstalled',
    defaultMessage: 'No skills installed',
  },
  noSkillsDescription: {
    id: 'skillsView.noSkillsDescription',
    defaultMessage:
      'Skills are SKILL.md files. Yours live in ~/.agents/skills, or in .agents/skills inside a project.',
  },
  noMatchingSkills: {
    id: 'skillsView.noMatchingSkills',
    defaultMessage: 'No matching skills found',
  },
  adjustSearchTerms: {
    id: 'skillsView.adjustSearchTerms',
    defaultMessage: 'Try adjusting your search terms',
  },
  skillsTitle: {
    id: 'skillsView.skillsTitle',
    defaultMessage: 'Skills',
  },
  addSkill: {
    id: 'skillsView.addSkill',
    defaultMessage: 'Add Skill',
  },
  skillsDescription: {
    id: 'skillsView.skillsDescription',
    defaultMessage: 'View installed skills that extend Goose capabilities. {shortcut} to search.',
  },
  searchSkillsPlaceholder: {
    id: 'skillsView.searchSkillsPlaceholder',
    defaultMessage: 'Search skills...',
  },
  originBuiltin: {
    id: 'skillsView.originBuiltin',
    defaultMessage: 'builtin',
  },
  originPlugin: {
    id: 'skillsView.originPlugin',
    defaultMessage: 'plugin',
  },
  originGlobal: {
    id: 'skillsView.originGlobal',
    defaultMessage: 'global',
  },
  originProject: {
    id: 'skillsView.originProject',
    defaultMessage: 'project',
  },
  shadowed: {
    id: 'skillsView.shadowed',
    defaultMessage: 'shadowed',
  },
  editSkill: {
    id: 'skillsView.editSkill',
    defaultMessage: 'Edit {name}',
  },
  removeSkill: {
    id: 'skillsView.removeSkill',
    defaultMessage: 'Remove {name}',
  },
  deleteTitle: {
    id: 'skillsView.deleteTitle',
    defaultMessage: 'Delete skill',
  },
  deleteConfirm: {
    id: 'skillsView.deleteConfirm',
    defaultMessage: 'Delete "{name}"?',
  },
  deleteDetail: {
    id: 'skillsView.deleteDetail',
    defaultMessage: 'This removes the whole directory: {path}',
  },
  deleted: {
    id: 'skillsView.deleted',
    defaultMessage: 'Skill deleted',
  },
  deleteFailed: {
    id: 'skillsView.deleteFailed',
    defaultMessage: 'Failed to delete skill',
  },
  saved: {
    id: 'skillsView.saved',
    defaultMessage: 'Skill saved',
  },
  savedButShadowed: {
    id: 'skillsView.savedButShadowed',
    defaultMessage: 'Saved, but not in use: {path} already answers to "{name}"',
  },
});

const ORIGIN_LABELS = {
  builtin: i18n.originBuiltin,
  plugin: i18n.originPlugin,
  global: i18n.originGlobal,
  project: i18n.originProject,
} as const;

function SkillItem({
  skill,
  onInspect,
  onEdit,
  onRemove,
}: {
  skill: FoundSkill;
  onInspect: () => void;
  onEdit: () => void;
  onRemove: () => void;
}) {
  const intl = useIntl();
  const origin = skillOrigin(skill.entry);
  const editable = isEditable(skill.entry);

  return (
    <Card className="py-2 px-4 mb-2 bg-background-primary border-none hover:bg-background-secondary transition-all duration-150">
      <div className="flex justify-between items-center gap-4">
        <button className="min-w-0 flex-1 text-left" onClick={onInspect}>
          <div className="flex items-center gap-2 mb-1">
            <h3 className="text-base truncate">{skill.entry.name}</h3>
            <span className="text-xs text-text-secondary shrink-0">
              {intl.formatMessage(ORIGIN_LABELS[origin])}
            </span>
            {skill.shadowedBy && (
              <span className="text-xs text-text-secondary shrink-0">
                · {intl.formatMessage(i18n.shadowed)}
              </span>
            )}
          </div>
          <p className="text-text-secondary text-sm line-clamp-2">{skill.entry.description}</p>
        </button>
        {editable && (
          <div className="flex items-center gap-2 shrink-0">
            <button
              className="text-text-secondary hover:text-text-primary"
              aria-label={intl.formatMessage(i18n.editSkill, { name: skill.entry.name })}
              onClick={onEdit}
            >
              <Pencil className="w-4 h-4" />
            </button>
            <button
              className="text-text-secondary hover:text-text-primary"
              aria-label={intl.formatMessage(i18n.removeSkill, { name: skill.entry.name })}
              onClick={onRemove}
            >
              <Trash2 className="w-4 h-4" />
            </button>
          </div>
        )}
      </div>
    </Card>
  );
}

function SkillSkeleton() {
  return (
    <Card className="p-2 mb-2 bg-background-primary">
      <div className="flex justify-between items-start gap-4">
        <div className="min-w-0 flex-1">
          <Skeleton className="h-5 w-3/4 mb-2" />
          <Skeleton className="h-4 w-full" />
        </div>
      </div>
    </Card>
  );
}

export default function SkillsView() {
  const intl = useIntl();
  const [skills, setSkills] = useState<FoundSkill[]>([]);
  const [loading, setLoading] = useState(true);
  const [showSkeleton, setShowSkeleton] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showContent, setShowContent] = useState(false);
  const [searchTerm, setSearchTerm] = useState('');
  const [inspected, setInspected] = useState<FoundSkill | null>(null);
  const [editing, setEditing] = useState<SourceEntry | null>(null);
  const [creating, setCreating] = useState(false);
  const [pendingRemoval, setPendingRemoval] = useState<FoundSkill | null>(null);
  const [isRemoving, setIsRemoving] = useState(false);

  const projectDir = getInitialWorkingDir();

  const filteredSkills = useMemo(() => {
    if (!searchTerm) return skills;
    const searchLower = searchTerm.toLowerCase();
    return skills.filter(
      (skill) =>
        skill.entry.name.toLowerCase().includes(searchLower) ||
        skill.entry.description.toLowerCase().includes(searchLower)
    );
  }, [skills, searchTerm]);

  const load = useCallback(async (): Promise<FoundSkill[]> => {
    const sources = await listSkillSources(projectDir, true);
    const found = withShadows(sources);
    setSkills(found);
    return found;
  }, [projectDir]);

  const loadSkills = useCallback(async () => {
    try {
      setLoading(true);
      setShowSkeleton(true);
      setShowContent(false);
      setError(null);
      await load();
    } catch (err) {
      setError(errorMessage(err, 'Failed to load skills'));
    } finally {
      setLoading(false);
    }
  }, [load]);

  useEffect(() => {
    loadSkills();
  }, [loadSkills]);

  useEffect(() => {
    if (!loading && showSkeleton) {
      const timer = setTimeout(() => {
        setShowSkeleton(false);
        setTimeout(() => setShowContent(true), 50);
      }, 300);
      return () => clearTimeout(timer);
    }
    return undefined;
  }, [loading, showSkeleton]);

  // A saved skill that lost a name collision is on disk but never loaded, so
  // silence here would read as success.
  const handleSaved = async (saved: SourceEntry) => {
    setCreating(false);
    setEditing(null);
    try {
      const found = await load();
      const shadowedBy = found.find((skill) => skill.entry.path === saved.path)?.shadowedBy;
      if (shadowedBy) {
        toastSuccess({
          title: saved.name,
          msg: intl.formatMessage(i18n.savedButShadowed, {
            path: shadowedBy,
            name: saved.name,
          }),
        });
        return;
      }
      toastSuccess({ title: saved.name, msg: intl.formatMessage(i18n.saved) });
    } catch (err) {
      setError(errorMessage(err, 'Failed to load skills'));
    }
  };

  const confirmRemoval = async () => {
    if (!pendingRemoval) return;
    setIsRemoving(true);
    try {
      await deleteSkill(pendingRemoval.entry.path);
      toastSuccess({
        title: pendingRemoval.entry.name,
        msg: intl.formatMessage(i18n.deleted),
      });
      setPendingRemoval(null);
      await load();
    } catch (err) {
      toastError({
        title: intl.formatMessage(i18n.deleteFailed),
        msg: errorMessage(err, 'Failed to delete skill'),
      });
    } finally {
      setIsRemoving(false);
    }
  };

  const renderContent = () => {
    if (loading || showSkeleton) {
      return (
        <div className="space-y-2">
          <SkillSkeleton />
          <SkillSkeleton />
          <SkillSkeleton />
        </div>
      );
    }

    if (error) {
      return (
        <div className="flex flex-col items-center justify-center h-full text-text-secondary">
          <AlertCircle className="h-12 w-12 text-red-500 mb-4" />
          <p className="text-lg mb-2">{intl.formatMessage(i18n.errorLoadingSkills)}</p>
          <p className="text-sm text-center mb-4">{error}</p>
          <Button onClick={loadSkills} variant="default">
            {intl.formatMessage(i18n.tryAgain)}
          </Button>
        </div>
      );
    }

    if (skills.length === 0) {
      return (
        <div className="flex flex-col justify-center pt-2 h-full">
          <p className="text-lg">{intl.formatMessage(i18n.noSkillsInstalled)}</p>
          <p className="text-sm text-text-secondary">
            {intl.formatMessage(i18n.noSkillsDescription)}
          </p>
        </div>
      );
    }

    if (filteredSkills.length === 0 && searchTerm) {
      return (
        <div className="flex flex-col items-center justify-center h-full text-text-secondary mt-4">
          <Zap className="h-12 w-12 mb-4" />
          <p className="text-lg mb-2">{intl.formatMessage(i18n.noMatchingSkills)}</p>
          <p className="text-sm">{intl.formatMessage(i18n.adjustSearchTerms)}</p>
        </div>
      );
    }

    return (
      <div className="space-y-2">
        {filteredSkills.map((skill) => (
          <SkillItem
            key={skill.entry.path}
            skill={skill}
            onInspect={() => setInspected(skill)}
            onEdit={() => setEditing(skill.entry)}
            onRemove={() => setPendingRemoval(skill)}
          />
        ))}
      </div>
    );
  };

  return (
    <MainPanelLayout>
      <div className="flex-1 flex flex-col min-h-0">
        <div className="bg-background-primary px-8 pb-8 pt-16">
          <div className="flex flex-col page-transition">
            <div className="flex justify-between items-center mb-1">
              <h1 className="text-4xl font-light">{intl.formatMessage(i18n.skillsTitle)}</h1>
              <Button
                variant="outline"
                size="sm"
                className="flex items-center gap-2"
                onClick={() => setCreating(true)}
              >
                <Plus className="w-4 h-4" />
                {intl.formatMessage(i18n.addSkill)}
              </Button>
            </div>
            <p className="text-sm text-text-secondary mb-1">
              {intl.formatMessage(i18n.skillsDescription, {
                shortcut: getSearchShortcutText(),
              })}
            </p>
          </div>
        </div>

        <div className="flex-1 min-h-0 relative px-8">
          <ScrollArea className="h-full">
            <SearchView
              onSearch={(term) => setSearchTerm(term)}
              placeholder={intl.formatMessage(i18n.searchSkillsPlaceholder)}
            >
              <div
                className={`h-full relative transition-all duration-300 ${
                  showContent || showSkeleton ? 'opacity-100 animate-in fade-in' : 'opacity-0'
                }`}
              >
                {renderContent()}
              </div>
            </SearchView>
          </ScrollArea>
        </div>
      </div>

      {inspected && (
        <SkillDetails skill={inspected} found={skills} onClose={() => setInspected(null)} />
      )}

      {(creating || editing) && (
        <SkillFormModal
          skill={editing ?? undefined}
          projectDir={projectDir}
          globalDirLabel="~/.agents/skills"
          projectDirLabel={`${projectDir}/.agents/skills`}
          onClose={() => {
            setCreating(false);
            setEditing(null);
          }}
          onSaved={handleSaved}
        />
      )}

      <ConfirmationModal
        isOpen={pendingRemoval !== null}
        title={intl.formatMessage(i18n.deleteTitle)}
        message={intl.formatMessage(i18n.deleteConfirm, {
          name: pendingRemoval?.entry.name ?? '',
        })}
        detail={intl.formatMessage(i18n.deleteDetail, {
          path: pendingRemoval?.entry.path ?? '',
        })}
        onConfirm={confirmRemoval}
        onCancel={() => setPendingRemoval(null)}
        isSubmitting={isRemoving}
        confirmVariant="destructive"
      />
    </MainPanelLayout>
  );
}
