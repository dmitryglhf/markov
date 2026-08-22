import { useState } from 'react';
import type { SourceEntry, SourceScope } from '@aaif/goose-sdk';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { defineMessages, useIntl } from '../../i18n';
import { errorMessage } from '../../utils/conversionUtils';
import { createSkill, updateSkill } from '../../acp/sources';
import { skillNameProblem, type SkillNameProblem } from './skillName';

const i18n = defineMessages({
  createTitle: {
    id: 'skillForm.createTitle',
    defaultMessage: 'New skill',
  },
  createDescription: {
    id: 'skillForm.createDescription',
    defaultMessage:
      'A skill is a SKILL.md file the agent loads when its description fits the task.',
  },
  editTitle: {
    id: 'skillForm.editTitle',
    defaultMessage: 'Edit skill',
  },
  nameLabel: {
    id: 'skillForm.nameLabel',
    defaultMessage: 'Name',
  },
  namePlaceholder: {
    id: 'skillForm.namePlaceholder',
    defaultMessage: 'lowercase-with-hyphens',
  },
  descriptionLabel: {
    id: 'skillForm.descriptionLabel',
    defaultMessage: 'Description',
  },
  descriptionPlaceholder: {
    id: 'skillForm.descriptionPlaceholder',
    defaultMessage: 'When the agent should reach for this skill',
  },
  descriptionRequired: {
    id: 'skillForm.descriptionRequired',
    defaultMessage: 'The description is how the agent decides to load a skill',
  },
  contentLabel: {
    id: 'skillForm.contentLabel',
    defaultMessage: 'Instructions',
  },
  contentPlaceholder: {
    id: 'skillForm.contentPlaceholder',
    defaultMessage: 'What the agent should do once this skill is loaded',
  },
  scopeLabel: {
    id: 'skillForm.scopeLabel',
    defaultMessage: 'Where should it live?',
  },
  scopeGlobal: {
    id: 'skillForm.scopeGlobal',
    defaultMessage: 'Everywhere',
  },
  scopeProject: {
    id: 'skillForm.scopeProject',
    defaultMessage: 'This project only',
  },
  nameEmpty: {
    id: 'skillForm.nameEmpty',
    defaultMessage: 'Skill name must not be empty',
  },
  nameTooLong: {
    id: 'skillForm.nameTooLong',
    defaultMessage: 'Names must be at most 64 characters',
  },
  nameCharset: {
    id: 'skillForm.nameCharset',
    defaultMessage: 'Names may only contain lowercase letters, digits, and hyphens',
  },
  nameHyphenEdge: {
    id: 'skillForm.nameHyphenEdge',
    defaultMessage: 'Names must not start or end with a hyphen',
  },
  cancel: {
    id: 'skillForm.cancel',
    defaultMessage: 'Cancel',
  },
  save: {
    id: 'skillForm.save',
    defaultMessage: 'Save',
  },
  saving: {
    id: 'skillForm.saving',
    defaultMessage: 'Saving...',
  },
});

const NAME_PROBLEM_MESSAGES: Record<SkillNameProblem, keyof typeof i18n> = {
  empty: 'nameEmpty',
  tooLong: 'nameTooLong',
  charset: 'nameCharset',
  hyphenEdge: 'nameHyphenEdge',
};

export type SkillFormModalProps = {
  skill?: SourceEntry;
  projectDir: string;
  globalDirLabel: string;
  projectDirLabel: string;
  onClose: () => void;
  onSaved: (skill: SourceEntry) => void;
};

export default function SkillFormModal({
  skill,
  projectDir,
  globalDirLabel,
  projectDirLabel,
  onClose,
  onSaved,
}: SkillFormModalProps) {
  const intl = useIntl();
  const [name, setName] = useState(skill?.name ?? '');
  const [description, setDescription] = useState(skill?.description ?? '');
  const [content, setContent] = useState(skill?.content ?? '');
  const [global, setGlobal] = useState(skill?.global ?? true);
  const [isSaving, setIsSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const trimmedName = name.trim();
  const nameProblem = skillNameProblem(trimmedName);
  const descriptionMissing = description.trim().length === 0;
  const canSave = !nameProblem && !descriptionMissing && !isSaving;

  const save = async () => {
    setIsSaving(true);
    setSaveError(null);
    try {
      const target: SourceScope = global
        ? { scope: 'global' }
        : { scope: 'projectDir', projectDir };
      const saved = skill
        ? await updateSkill(skill.path, trimmedName, description.trim(), content)
        : await createSkill(trimmedName, description.trim(), content, target);
      onSaved(saved);
    } catch (error) {
      setSaveError(errorMessage(error, 'Failed to save skill'));
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <Dialog open={true} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="w-[80vw] max-w-[80vw] sm:max-w-[80vw] max-h-[90vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>{intl.formatMessage(skill ? i18n.editTitle : i18n.createTitle)}</DialogTitle>
          <DialogDescription>
            {skill ? skill.path : intl.formatMessage(i18n.createDescription)}
          </DialogDescription>
        </DialogHeader>

        <div className="flex-1 overflow-y-auto space-y-4 pt-2 pb-4">
          <div className="space-y-1">
            <label className="text-sm text-text-secondary" htmlFor="skill-name">
              {intl.formatMessage(i18n.nameLabel)}
            </label>
            <Input
              id="skill-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder={intl.formatMessage(i18n.namePlaceholder)}
            />
            {trimmedName.length > 0 && nameProblem && (
              <p className="text-sm text-red-500">
                {intl.formatMessage(i18n[NAME_PROBLEM_MESSAGES[nameProblem]])}
              </p>
            )}
          </div>

          <div className="space-y-1">
            <label className="text-sm text-text-secondary" htmlFor="skill-description">
              {intl.formatMessage(i18n.descriptionLabel)}
            </label>
            <Input
              id="skill-description"
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              placeholder={intl.formatMessage(i18n.descriptionPlaceholder)}
            />
            <p className="text-sm text-text-secondary">
              {intl.formatMessage(i18n.descriptionRequired)}
            </p>
          </div>

          {!skill && (
            <div className="space-y-1">
              <p className="text-sm text-text-secondary">{intl.formatMessage(i18n.scopeLabel)}</p>
              <div className="flex gap-2">
                <Button
                  variant={global ? 'default' : 'outline'}
                  size="sm"
                  onClick={() => setGlobal(true)}
                  title={globalDirLabel}
                >
                  {intl.formatMessage(i18n.scopeGlobal)}
                </Button>
                <Button
                  variant={global ? 'outline' : 'default'}
                  size="sm"
                  onClick={() => setGlobal(false)}
                  title={projectDirLabel}
                >
                  {intl.formatMessage(i18n.scopeProject)}
                </Button>
              </div>
              <p className="font-mono text-xs text-text-secondary">
                {global ? globalDirLabel : projectDirLabel}
              </p>
            </div>
          )}

          <div className="space-y-1">
            <label className="text-sm text-text-secondary" htmlFor="skill-content">
              {intl.formatMessage(i18n.contentLabel)}
            </label>
            <textarea
              id="skill-content"
              value={content}
              className="w-full h-80 border rounded-md p-2 text-sm resize-none bg-background-primary text-text-primary border-border-primary focus:outline-none focus:ring-2 focus:ring-blue-500"
              onChange={(event) => setContent(event.target.value)}
              placeholder={intl.formatMessage(i18n.contentPlaceholder)}
            />
          </div>

          {saveError && <p className="text-sm text-red-500">{saveError}</p>}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            {intl.formatMessage(i18n.cancel)}
          </Button>
          <Button onClick={save} disabled={!canSave}>
            {intl.formatMessage(isSaving ? i18n.saving : i18n.save)}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
