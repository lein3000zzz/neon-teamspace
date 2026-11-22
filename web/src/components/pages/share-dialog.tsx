import { useQuery } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { hasCode } from "@/api/errors";
import {
  breadcrumbsQuery,
  sharesQuery,
  useAddShare,
  useChangeShare,
  useRemoveShare,
  type Share,
  type ShareAccess,
} from "@/api/pages";
import { membersQuery, type Member, type WorkspaceSummary } from "@/api/workspaces";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Field } from "@/components/forms/field";
import { Chip } from "@/components/nodes/chip";
import { HueDot } from "@/components/nodes/hue-dot";
import { useMe } from "@/components/shell/signed-in";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { NativeSelect } from "@/components/ui/native-select";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";

const copy = strings.pages.share;
const ACCESSES = ["can_edit", "can_view"] as const satisfies readonly ShareAccess[];

type ShareDialogProps = {
  workspace: WorkspaceSummary;
  page: { id: string; title: string };
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

/**
 * Who has a share on a page: those set on it, which change here, then those
 * set on a page above it, which hold here too and change there. Guests are
 * marked, as they reach nothing else in the workspace. Only those who edit a
 * page, and are not guests, open it.
 */
export function ShareDialog({ workspace, page, open, onOpenChange }: ShareDialogProps) {
  const title = titleOf(page);
  const shares = useQuery({ ...sharesQuery(workspace.id, page.id), enabled: open });
  const members = useQuery({ ...membersQuery(workspace.id), enabled: open });
  const crumbs = useQuery({ ...breadcrumbsQuery(workspace.id, page.id), enabled: open });
  const [done, setDone] = useState("");

  const roles = new Map(members.data?.map((member) => [member.user.id, member.role]));
  const titles = new Map(crumbs.data?.map((crumb) => [crumb.id, titleOf(crumb)]));
  const own = shares.data?.filter((share) => !share.inherited) ?? [];
  const inherited = shares.data?.filter((share) => share.inherited) ?? [];

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setDone("");
        onOpenChange(next);
      }}
    >
      <DialogContent closeLabel={copy.done}>
        <DialogTitle>{copy.title(title)}</DialogTitle>
        <DialogDescription>{copy.lede}</DialogDescription>
        <output className="block text-sm text-muted-foreground empty:hidden">{done}</output>
        {members.data && shares.data && (
          <AddShareForm
            workspaceId={workspace.id}
            pageId={page.id}
            members={members.data}
            taken={new Set(own.map((share) => share.user.id))}
            onShared={(name) => setDone(copy.shared(name))}
          />
        )}
        <section aria-label={copy.list} className="grid gap-2">
          <h3 className="text-sm font-semibold">{copy.list}</h3>
          {(shares.isPending || members.isPending) && <Loading />}
          {shares.isError && (
            <ErrorNotice
              error={shares.error}
              retrying={shares.isFetching}
              onRetry={() => void shares.refetch()}
            />
          )}
          {shares.data?.length === 0 && (
            <p className="text-sm text-muted-foreground">{copy.none}</p>
          )}
          {shares.data !== undefined && shares.data.length > 0 && (
            <ul className="grid gap-2">
              {own.map((share) => (
                <ShareRow
                  key={share.user.id}
                  workspaceId={workspace.id}
                  pageId={page.id}
                  share={share}
                  guest={roles.get(share.user.id) === "guest"}
                  onDone={setDone}
                />
              ))}
              {inherited.map((share, index) => {
                // A share set on a page the caller may not see comes without its page.
                const setOn = share.page_id ?? undefined;
                const from = setOn === undefined ? undefined : titles.get(setOn);
                return (
                  <li
                    key={`${share.page_id ?? index}:${share.user.id}`}
                    className="grid gap-1 rounded-lg border border-border bg-card p-3"
                  >
                    <Person share={share} guest={roles.get(share.user.id) === "guest"} />
                    <p className="text-xs text-muted-foreground">
                      {copy.accesses[share.access]} ·{" "}
                      {from === undefined ? copy.fromAbove : copy.from(from)}
                    </p>
                  </li>
                );
              })}
            </ul>
          )}
        </section>
      </DialogContent>
    </Dialog>
  );
}

function Person({ share, guest }: { share: Share; guest: boolean }) {
  return (
    <div className="grid min-w-0 gap-0.5">
      <p className="flex min-w-0 flex-wrap items-center gap-2 text-sm font-medium">
        <HueDot hue={share.user.hue} />
        <span className="truncate">{share.user.display_name}</span>
        {guest && <Chip>{copy.guest}</Chip>}
      </p>
      <p className="truncate text-xs text-muted-foreground">{share.user.email}</p>
    </div>
  );
}

type ShareRowProps = {
  workspaceId: string;
  pageId: string;
  share: Share;
  guest: boolean;
  onDone: (message: string) => void;
};

function ShareRow({ workspaceId, pageId, share, guest, onDone }: ShareRowProps) {
  const change = useChangeShare(workspaceId, pageId);
  const remove = useRemoveShare(workspaceId, pageId);
  const name = share.user.display_name;
  const failure = change.error ?? remove.error;
  return (
    <li className="grid gap-3 rounded-lg border border-border bg-card p-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
      <Person share={share} guest={guest} />
      <div className="flex flex-wrap items-center gap-2">
        <div className="w-36">
          <NativeSelect
            aria-label={copy.accessOf(name)}
            value={share.access}
            disabled={change.isPending || remove.isPending}
            onChange={(event) =>
              change.mutate({ userId: share.user.id, access: event.target.value as ShareAccess })
            }
          >
            {ACCESSES.map((access) => (
              <option key={access} value={access}>
                {copy.accesses[access]}
              </option>
            ))}
          </NativeSelect>
        </div>
        <Button
          variant="outline"
          size="sm"
          aria-label={copy.removeLabel(name)}
          disabled={change.isPending || remove.isPending}
          onClick={() =>
            remove.mutate(share.user.id, { onSuccess: () => onDone(copy.removed(name)) })
          }
        >
          {copy.remove}
        </Button>
      </div>
      {failure !== null && (
        <div className="sm:col-span-2">
          <ErrorNotice error={failure} />
        </div>
      )}
    </li>
  );
}

type AddShareFormProps = {
  workspaceId: string;
  pageId: string;
  members: Member[];
  /** Who has a share on this page already. */
  taken: ReadonlySet<string>;
  onShared: (name: string) => void;
};

function AddShareForm({ workspaceId, pageId, members, taken, onShared }: AddShareFormProps) {
  const me = useMe();
  const add = useAddShare(workspaceId, pageId);
  const [userId, setUserId] = useState("");
  const [access, setAccess] = useState<ShareAccess>("can_view");
  const [missing, setMissing] = useState(false);
  const candidates = members.filter(
    (member) => member.user.id !== me.id && !taken.has(member.user.id),
  );

  if (candidates.length === 0) {
    return <p className="text-sm text-muted-foreground">{copy.nobody}</p>;
  }

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const person = candidates.find((member) => member.user.id === userId);
    if (person === undefined) {
      setMissing(true);
      return;
    }
    add.mutate(
      { user_id: person.user.id, access },
      {
        onSuccess: () => {
          setUserId("");
          onShared(person.user.display_name);
        },
      },
    );
  };

  return (
    <form
      aria-label={copy.add}
      noValidate
      onSubmit={submit}
      className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_9rem_auto] sm:items-end"
    >
      <Field label={copy.person} error={missing ? strings.validation.required : undefined}>
        {(control) => (
          <NativeSelect
            {...control}
            value={userId}
            onChange={(event) => {
              setMissing(false);
              setUserId(event.target.value);
            }}
          >
            <option value="">{copy.choose}</option>
            {candidates.map((member) => (
              <option key={member.user.id} value={member.user.id}>
                {member.role === "guest"
                  ? `${member.user.display_name} (${copy.guest})`
                  : member.user.display_name}
              </option>
            ))}
          </NativeSelect>
        )}
      </Field>
      <Field label={copy.access}>
        {(control) => (
          <NativeSelect
            {...control}
            value={access}
            onChange={(event) => setAccess(event.target.value as ShareAccess)}
          >
            {ACCESSES.map((choice) => (
              <option key={choice} value={choice}>
                {copy.accesses[choice]}
              </option>
            ))}
          </NativeSelect>
        )}
      </Field>
      <Button type="submit" disabled={add.isPending}>
        {add.isPending ? copy.pending : copy.submit}
      </Button>
      {add.isError && (
        <div className="sm:col-span-3">
          {hasCode(add.error, "already_shared") ? (
            <Alert tone="error" live="assertive">
              <p className="text-foreground">{copy.already}</p>
            </Alert>
          ) : (
            <ErrorNotice error={add.error} />
          )}
        </div>
      )}
    </form>
  );
}
