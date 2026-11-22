import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Plus } from "lucide-react";
import { useState, type FormEvent } from "react";
import type { User } from "@/api/account";
import { usersQuery, useCreateUser, useSendVerification, useUpdateUser } from "@/api/admin";
import { authConfigQuery } from "@/api/auth";
import type { Schemas } from "@/api/client";
import { ApiError, hasCode } from "@/api/errors";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, errorText, Loading } from "@/components/feedback/error-notice";
import { Field, focusFirstInvalid } from "@/components/forms/field";
import { Chip, ChipList } from "@/components/nodes/chip";
import { ItemActions, ItemCard, PageSection } from "@/components/settings/page-section";
import { useMe } from "@/components/shell/signed-in";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { formatDate } from "@/lib/format";
import { strings } from "@/lib/strings";
import { displayNameProblem, emailProblem, passwordProblem } from "@/lib/validation";

const copy = strings.admin.users;

/** Every account: add one, change roles, disable and enable, vouch for an address. */
export function UsersPage() {
  const users = useInfiniteQuery(usersQuery);
  const [adding, setAdding] = useState(false);
  const [added, setAdded] = useState<string>();
  const list = users.data?.pages.flatMap((page) => page.users) ?? [];

  return (
    <div className="grid gap-10">
      <PageSection
        title={copy.add}
        hint={copy.addHint}
        actions={
          !adding && (
            <Button
              variant="outline"
              onClick={() => {
                setAdded(undefined);
                setAdding(true);
              }}
            >
              <Plus aria-hidden />
              {copy.add}
            </Button>
          )
        }
      >
        <output className="block text-sm text-muted-foreground empty:hidden">
          {added === undefined ? "" : copy.added(added)}
        </output>
        {adding && (
          <CreateUserForm
            onCancel={() => setAdding(false)}
            onCreated={(user) => {
              setAdding(false);
              setAdded(user.email);
            }}
          />
        )}
      </PageSection>

      <PageSection title={copy.list}>
        {users.isPending && <Loading />}
        {users.isError && (
          <ErrorNotice
            error={users.error}
            retrying={users.isFetching}
            onRetry={() => void users.refetch()}
          />
        )}
        {users.isSuccess && list.length === 0 && (
          <p className="text-sm text-muted-foreground">{copy.none}</p>
        )}
        {list.length > 0 && (
          <ul className="grid gap-3">
            {list.map((user) => (
              <UserCard key={user.id} user={user} />
            ))}
          </ul>
        )}
        {users.hasNextPage && (
          <Button
            variant="outline"
            className="justify-self-center"
            disabled={users.isFetchingNextPage}
            onClick={() => void users.fetchNextPage()}
          >
            {copy.loadMore}
          </Button>
        )}
      </PageSection>
    </div>
  );
}

type Problems = {
  email?: string | undefined;
  password?: string | undefined;
  display_name?: string | undefined;
};

/** Which field a refusal from the server is about. */
const FIELD_OF: Record<string, keyof Problems> = {
  email_taken: "email",
  invalid_email: "email",
  weak_password: "password",
};

function CreateUserForm({
  onCreated,
  onCancel,
}: {
  onCreated: (user: User) => void;
  onCancel: () => void;
}) {
  const config = useQuery(authConfigQuery);
  const create = useCreateUser();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [role, setRole] = useState<Schemas["Role"]>("user");
  const [problems, setProblems] = useState<Problems>({});
  const minLength = config.data?.min_password_length;

  const error = create.error;
  const serverField = error instanceof ApiError ? FIELD_OF[error.code] : undefined;
  const shown = {
    ...problems,
    ...(serverField === undefined ? {} : { [serverField]: errorText(error) }),
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    create.reset();
    const found: Problems = {
      email: emailProblem(email),
      password: passwordProblem(password, minLength ?? 1),
      display_name: displayNameProblem(displayName),
    };
    setProblems(found);
    const invalid = (Object.keys(found) as (keyof Problems)[]).filter((key) => found[key]);
    if (invalid.length > 0) {
      focusFirstInvalid(event.currentTarget, invalid);
      return;
    }
    const name = displayName.trim();
    create.mutate(
      { email: email.trim(), password, role, ...(name === "" ? {} : { display_name: name }) },
      { onSuccess: onCreated },
    );
  };

  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.add}
      className="grid max-w-xl gap-4 rounded-lg border border-border bg-card p-4"
    >
      {create.isError && serverField === undefined && <ErrorNotice error={error} />}
      <Field label={strings.auth.email} error={shown.email}>
        {(control) => (
          <Input
            {...control}
            name="email"
            type="email"
            autoComplete="off"
            value={email}
            onChange={(event) => setEmail(event.target.value)}
          />
        )}
      </Field>
      <Field label={copy.displayName} hint={copy.displayNameHint} error={shown.display_name}>
        {(control) => (
          <Input
            {...control}
            name="display_name"
            autoComplete="off"
            value={displayName}
            onChange={(event) => setDisplayName(event.target.value)}
          />
        )}
      </Field>
      <Field
        label={strings.auth.password}
        hint={minLength === undefined ? undefined : copy.passwordHint(minLength)}
        error={shown.password}
      >
        {(control) => (
          <Input
            {...control}
            name="password"
            type="password"
            autoComplete="new-password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
          />
        )}
      </Field>
      <Field label={copy.role}>
        {(control) => (
          <NativeSelect
            {...control}
            name="role"
            value={role}
            onChange={(event) => setRole(event.target.value === "admin" ? "admin" : "user")}
          >
            <option value="user">{copy.roles.user}</option>
            <option value="admin">{copy.roles.admin}</option>
          </NativeSelect>
        )}
      </Field>
      <div className="flex flex-wrap gap-2">
        <Button type="submit" disabled={create.isPending}>
          {create.isPending ? copy.pending : copy.submit}
        </Button>
        <Button type="button" variant="ghost" onClick={onCancel}>
          {strings.confirm.cancel}
        </Button>
      </div>
    </form>
  );
}

type Change = { patch: Schemas["UpdateUser"]; title: string; lede: string; action: string };

/** What a change does, said before it is made. */
function describeChange(user: User, patch: Schemas["UpdateUser"], self: boolean): Change {
  const confirm = copy.confirm;
  if (patch.role === "admin") {
    return {
      patch,
      title: confirm.makeAdmin.title(user.email),
      lede: confirm.makeAdmin.lede,
      action: copy.makeAdmin,
    };
  }
  if (patch.role === "user") {
    return {
      patch,
      title: confirm.makeUser.title(user.email),
      lede: self ? `${confirm.makeUser.lede} ${confirm.makeUser.self}` : confirm.makeUser.lede,
      action: copy.makeUser,
    };
  }
  if (patch.disabled === true) {
    return {
      patch,
      title: confirm.disable.title(user.email),
      lede: self ? `${confirm.disable.lede} ${confirm.disable.self}` : confirm.disable.lede,
      action: copy.disable,
    };
  }
  return {
    patch,
    title: confirm.enable.title(user.email),
    lede: confirm.enable.lede,
    action: copy.enable,
  };
}

function UserCard({ user }: { user: User }) {
  const me = useMe();
  const self = user.id === me.id;
  const update = useUpdateUser();
  const sendLink = useSendVerification();
  const [pending, setPending] = useState<Change>();
  const [done, setDone] = useState<string>();

  const apply = (patch: Schemas["UpdateUser"]) => {
    setDone(undefined);
    sendLink.reset();
    update.mutate(
      { id: user.id, ...patch },
      { onSuccess: () => setDone(copy.updated(user.email)) },
    );
  };
  const ask = (patch: Schemas["UpdateUser"]) => setPending(describeChange(user, patch, self));
  const label = (action: string) => copy.actionLabel(action, user.email);
  const busy = update.isPending || sendLink.isPending;

  return (
    <ItemCard>
      <div className="grid min-w-0 gap-1.5">
        <p className="font-medium break-words">{user.display_name}</p>
        <p className="text-sm break-all text-muted-foreground">{user.email}</p>
        <ChipList>
          {user.role === "admin" && <Chip tone="strong">{strings.admin.users.roles.admin}</Chip>}
          {self && <Chip>{copy.you}</Chip>}
          {user.disabled_at && <Chip>{copy.disabled}</Chip>}
          {!user.email_verified_at && <Chip>{copy.unverified}</Chip>}
          <Chip>{copy.joined(formatDate(user.created_at))}</Chip>
        </ChipList>
        {update.isError &&
          (hasCode(update.error, "last_admin") ? (
            <Alert tone="error" live="assertive">
              <p className="text-foreground">{copy.lastAdmin}</p>
            </Alert>
          ) : (
            <ErrorNotice error={update.error} />
          ))}
        {sendLink.isError && <ErrorNotice error={sendLink.error} />}
        <output className="block text-sm text-muted-foreground empty:hidden">
          {sendLink.isSuccess ? copy.linkSent(user.email) : (done ?? "")}
        </output>
      </div>
      <ItemActions>
        <Button
          variant="outline"
          size="sm"
          disabled={busy}
          aria-label={label(user.role === "admin" ? copy.makeUser : copy.makeAdmin)}
          onClick={() => ask({ role: user.role === "admin" ? "user" : "admin" })}
        >
          {user.role === "admin" ? copy.makeUser : copy.makeAdmin}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={busy}
          aria-label={label(user.disabled_at ? copy.enable : copy.disable)}
          onClick={() => ask({ disabled: !user.disabled_at })}
        >
          {user.disabled_at ? copy.enable : copy.disable}
        </Button>
        {!user.email_verified_at && (
          <>
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              aria-label={label(copy.markVerified)}
              onClick={() => apply({ email_verified: true })}
            >
              {copy.markVerified}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={busy}
              aria-label={label(copy.sendLink)}
              onClick={() => {
                setDone(undefined);
                update.reset();
                sendLink.mutate(user.id);
              }}
            >
              {copy.sendLink}
            </Button>
          </>
        )}
      </ItemActions>
      <ConfirmDialog
        open={pending !== undefined}
        title={pending?.title}
        description={pending?.lede}
        confirm={pending?.action ?? ""}
        onCancel={() => setPending(undefined)}
        onConfirm={() => {
          if (pending) {
            apply(pending.patch);
          }
          setPending(undefined);
        }}
      />
    </ItemCard>
  );
}
