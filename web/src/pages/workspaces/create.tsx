import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Plus } from "lucide-react";
import { useState, type FormEvent } from "react";
import { hasCode } from "@/api/errors";
import { useCreateWorkspace, workspacesQuery } from "@/api/workspaces";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Field, focusFirstInvalid } from "@/components/forms/field";
import { PageHeader } from "@/components/shell/page-header";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { strings } from "@/lib/strings";
import { iconProblem, nameProblem, slugProblem } from "@/lib/validation";

const copy = strings.workspaces.create;

type Problems = {
  name?: string | undefined;
  icon?: string | undefined;
  slug?: string | undefined;
};

/** A new workspace: its name, and optionally an icon and an address of its own. */
export function CreateWorkspacePage() {
  const list = useQuery(workspacesQuery);
  return (
    <>
      <PageHeader eyebrow={copy.eyebrow} title={copy.title} lede={copy.lede} />
      {list.isPending && <Loading />}
      {list.isError && (
        <ErrorNotice
          error={list.error}
          retrying={list.isFetching}
          onRetry={() => void list.refetch()}
        />
      )}
      {list.data &&
        (list.data.can_create ? (
          <CreateWorkspaceForm />
        ) : (
          <Alert live="polite">
            <p className="text-foreground">{copy.adminsOnly}</p>
          </Alert>
        ))}
    </>
  );
}

function CreateWorkspaceForm() {
  const navigate = useNavigate();
  const create = useCreateWorkspace();
  const [name, setName] = useState("");
  const [icon, setIcon] = useState("");
  const [slug, setSlug] = useState("");
  const [problems, setProblems] = useState<Problems>({});

  const error = create.error;
  const refusedSlug = hasCode(error, "slug_taken") ? copy.slugTaken : undefined;
  // The rule changed since the page loaded: an admin left creating workspaces to admins.
  const adminsOnly = hasCode(error, "forbidden");
  const shown = { ...problems, ...(refusedSlug === undefined ? {} : { slug: refusedSlug }) };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    create.reset();
    const found: Problems = {
      name: nameProblem(name),
      icon: iconProblem(icon),
      slug: slugProblem(slug),
    };
    setProblems(found);
    const invalid = (Object.keys(found) as (keyof Problems)[]).filter((key) => found[key]);
    if (invalid.length > 0) {
      focusFirstInvalid(event.currentTarget, invalid);
      return;
    }
    const typedIcon = icon.trim();
    const typedSlug = slug.trim().toLowerCase();
    create.mutate(
      {
        name: name.trim(),
        ...(typedIcon === "" ? {} : { icon: typedIcon }),
        ...(typedSlug === "" ? {} : { slug: typedSlug }),
      },
      {
        onSuccess: (workspace) =>
          void navigate({ to: "/w/$slug", params: { slug: workspace.slug } }),
      },
    );
  };

  return (
    <form noValidate onSubmit={submit} aria-label={copy.title} className="grid max-w-md gap-4">
      {adminsOnly && (
        <Alert tone="error" live="assertive">
          <p className="text-foreground">{copy.adminsOnly}</p>
        </Alert>
      )}
      {create.isError && !adminsOnly && refusedSlug === undefined && <ErrorNotice error={error} />}
      <Field label={copy.name} error={shown.name}>
        {(control) => (
          <Input
            {...control}
            name="name"
            autoComplete="off"
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
        )}
      </Field>
      <Field label={copy.icon} hint={copy.iconHint} error={shown.icon}>
        {(control) => (
          <Input
            {...control}
            name="icon"
            autoComplete="off"
            className="max-w-32"
            value={icon}
            onChange={(event) => setIcon(event.target.value)}
          />
        )}
      </Field>
      <Field label={copy.slug} hint={copy.slugHint} error={shown.slug}>
        {(control) => (
          <Input
            {...control}
            name="slug"
            autoComplete="off"
            autoCapitalize="off"
            spellCheck={false}
            className="font-mono"
            value={slug}
            onChange={(event) => setSlug(event.target.value)}
          />
        )}
      </Field>
      <div>
        <Button type="submit" disabled={create.isPending}>
          <Plus aria-hidden />
          {create.isPending ? copy.pending : copy.submit}
        </Button>
      </div>
    </form>
  );
}
