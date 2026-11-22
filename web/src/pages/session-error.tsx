import { useRouter, type ErrorComponentProps } from "@tanstack/react-router";
import { AuthCard, AuthLayout } from "@/components/auth/auth-layout";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { strings } from "@/lib/strings";

/**
 * Shown when asking who is signed in failed, usually because the server is
 * down: without an answer no signed-in page can show, so this stands in for
 * the shell, with a retry that loads the page again.
 */
export function SessionErrorPage({ error }: ErrorComponentProps) {
  const router = useRouter();
  return (
    <AuthLayout>
      <AuthCard title={strings.errors.session}>
        <ErrorNotice error={error} onRetry={() => void router.invalidate()} />
      </AuthCard>
    </AuthLayout>
  );
}
