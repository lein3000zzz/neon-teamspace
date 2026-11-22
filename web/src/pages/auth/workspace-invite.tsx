import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useParams } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { meQuery } from "@/api/account";
import { authConfigQuery } from "@/api/auth";
import { hasCode } from "@/api/errors";
import { useJoinWithLink, usePreviewInviteLink, type InviteLinkPreview } from "@/api/workspaces";
import { AuthCard, AuthFooterLine, authLink } from "@/components/auth/auth-layout";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { EntityMark } from "@/components/nodes/entity-mark";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { formatDateTime } from "@/lib/format";
import {
  heldInviteToken,
  holdInviteToken,
  noteInviteSignIn,
  releaseInviteToken,
} from "@/lib/links";
import { strings } from "@/lib/strings";
import { useSendOnce } from "./link-pages";

const copy = strings.workspaceInvite;

/**
 * Opened from a workspace invite link, /workspace-invite/TOKEN. The token is
 * kept for this tab and taken out of the address, which then reads
 * /workspace-invite, the page that carries on from it, signing in included.
 * Where the browser keeps nothing for the tab, the token stays in the address
 * for the way back from signing in.
 */
export function WorkspaceInviteLinkPage() {
  const { token } = useParams({ from: "/_auth/workspace-invite/$token" });
  const navigate = useNavigate();
  const [kept] = useState(() => holdInviteToken(token));

  useEffect(() => {
    if (kept) {
      void navigate({ to: "/workspace-invite", replace: true });
    }
  }, [kept, navigate]);

  if (kept) {
    return (
      <AuthCard title={copy.title}>
        <Loading label={copy.loading} />
      </AuthCard>
    );
  }
  return <InviteLanding token={token} back={`/workspace-invite/${encodeURIComponent(token)}`} />;
}

/** The same page once the token is out of the address. */
export function WorkspaceInvitePage() {
  const [token] = useState(heldInviteToken);
  if (token === undefined) {
    return <AuthCard title={copy.missingTitle} lede={copy.missingLede} footer={<HomeLine />} />;
  }
  return <InviteLanding token={token} back="/workspace-invite" />;
}

/**
 * Signed out: the way to sign in, and back here. Signed in: what the link
 * admits to, the workspace by name and the role, and joining it with one press.
 */
function InviteLanding({ token, back }: { token: string; back: string }) {
  const me = useQuery(meQuery);
  const signedOut = me.data === undefined ? undefined : me.data === null;
  useEffect(() => {
    if (signedOut !== undefined) {
      noteInviteSignIn(signedOut);
    }
  }, [signedOut]);

  if (me.isPending) {
    return (
      <AuthCard title={copy.title}>
        <Loading />
      </AuthCard>
    );
  }
  if (me.isError) {
    return (
      <AuthCard title={copy.title}>
        <ErrorNotice error={me.error} retrying={me.isFetching} onRetry={() => void me.refetch()} />
      </AuthCard>
    );
  }
  if (me.data === null) {
    return <SignInFirst back={back} />;
  }
  return <Preview token={token} email={me.data.email} />;
}

function SignInFirst({ back }: { back: string }) {
  const config = useQuery(authConfigQuery);
  return (
    <AuthCard
      title={copy.title}
      lede={copy.signedOutLede}
      footer={
        config.data &&
        config.data.registration !== "closed" && (
          <AuthFooterLine question={strings.auth.signIn.noAccount}>
            <Link to="/register" className={authLink}>
              {strings.auth.signIn.createOne}
            </Link>
          </AuthFooterLine>
        )
      }
    >
      <Button asChild size="lg">
        <Link to="/login" search={{ next: back }}>
          {copy.signIn}
        </Link>
      </Button>
    </AuthCard>
  );
}

function Preview({ token, email }: { token: string; email: string }) {
  const { mutate, data, error, isError } = usePreviewInviteLink();
  useSendOnce(token, mutate);

  if (isError) {
    if (hasCode(error, "invalid_invite_link")) {
      return <BrokenLink />;
    }
    return (
      <AuthCard title={copy.title} footer={<HomeLine />}>
        <ErrorNotice error={error} onRetry={() => mutate(token)} />
      </AuthCard>
    );
  }
  if (data === undefined) {
    return (
      <AuthCard title={copy.title}>
        <Loading label={copy.loading} />
      </AuthCard>
    );
  }
  return <JoinCard token={token} preview={data} email={email} />;
}

function JoinCard({
  token,
  preview,
  email,
}: {
  token: string;
  preview: InviteLinkPreview;
  email: string;
}) {
  const navigate = useNavigate();
  const join = useJoinWithLink();
  const { workspace } = preview;
  const already = preview.member_role ?? undefined;
  const open = () => {
    releaseInviteToken();
    void navigate({ to: "/w/$slug", params: { slug: workspace.slug } });
  };

  if (hasCode(join.error, "invalid_invite_link")) {
    return <BrokenLink />;
  }
  const title = (
    <span className="flex min-w-0 items-center gap-3">
      <EntityMark name={workspace.name} icon={workspace.icon} hue={workspace.hue} size="lg" />
      <span className="break-words">{workspace.name}</span>
    </span>
  );

  if (already !== undefined || hasCode(join.error, "already_member")) {
    return (
      <AuthCard
        title={title}
        lede={copy.already(workspace.name, strings.roleNames[already ?? preview.role])}
        footer={<p>{copy.signedInAs(email)}</p>}
      >
        <Button size="lg" onClick={open}>
          {copy.open(workspace.name)}
        </Button>
      </AuthCard>
    );
  }
  return (
    <AuthCard
      title={title}
      lede={`${copy.lede(strings.roleNames[preview.role])} ${copy.ends(formatDateTime(preview.expires_at))}`}
      footer={<p>{copy.signedInAs(email)}</p>}
    >
      {join.isError &&
        (hasCode(join.error, "member_limit") ? (
          <Alert tone="error" live="assertive">
            <p className="text-foreground">{copy.full}</p>
          </Alert>
        ) : (
          <ErrorNotice error={join.error} />
        ))}
      <Button
        size="lg"
        disabled={join.isPending}
        onClick={() => join.mutate(token, { onSuccess: open })}
      >
        {join.isPending ? copy.joining : copy.join(workspace.name)}
      </Button>
      <Button asChild variant="ghost">
        <Link to="/" onClick={releaseInviteToken}>
          {copy.home}
        </Link>
      </Button>
    </AuthCard>
  );
}

function BrokenLink() {
  useEffect(releaseInviteToken, []);
  return <AuthCard title={copy.brokenTitle} lede={copy.brokenLede} footer={<HomeLine />} />;
}

function HomeLine() {
  return (
    <p>
      <Link to="/" className={authLink} onClick={releaseInviteToken}>
        {copy.home}
      </Link>
    </p>
  );
}
