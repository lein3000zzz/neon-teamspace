import { queryOptions, useMutation, useQueryClient } from "@tanstack/react-query";
import { forgetPrivateData, refreshSession, type Me } from "./account";
import { call, send } from "./call";
import { api, type Schemas } from "./client";
import { keys } from "./keys";

export type AuthConfig = Schemas["AuthConfig"];
export type Registration = Schemas["Registration"];

/**
 * Which sign-in and sign-up forms are on, and what a password needs. It
 * changes only when an admin saves the settings, which refreshes it.
 */
export const authConfigQuery = queryOptions({
  queryKey: keys.authConfig,
  queryFn: ({ signal }) => call(api.GET("/api/v1/auth/config", { signal })),
  staleTime: 60 * 1000,
});

/** Signs in: the server sets the session cookie, then the new user is read. */
export function useLogin() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (body: Schemas["LoginRequest"]) => {
      const signedIn = await call(api.POST("/api/v1/auth/login", { body }));
      await refreshSession(queryClient);
      return signedIn;
    },
  });
}

/** What signing up led to. */
export type SignUp =
  /** Signed straight in. */
  | { kind: "signed-in" }
  /** The account waits for the link mailed to it; `sent` is false when the mail failed. */
  | { kind: "verify"; email: string; sent: boolean }
  /** The account exists, but signing in right after failed, so the person signs in by hand. */
  | { kind: "created"; email: string; error: unknown };

/**
 * Creates the account, then signs straight in with the same password, unless
 * the address must be verified first. The server never signs anyone in at
 * sign-up; this is the form doing it for them.
 */
export function useRegister() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (body: Schemas["RegisterRequest"]): Promise<SignUp> => {
      const registered = await call(api.POST("/api/v1/auth/register", { body }));
      const email = registered.user.email;
      if (registered.must_verify) {
        return { kind: "verify", email, sent: registered.sent };
      }
      try {
        await call(api.POST("/api/v1/auth/login", { body: { email, password: body.password } }));
        await refreshSession(queryClient);
        return { kind: "signed-in" };
      } catch (error) {
        return { kind: "created", email, error };
      }
    },
  });
}

/** Signs out: nobody is signed in any more, and nothing of theirs stays in memory. */
export function useLogout() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => send(api.POST("/api/v1/auth/logout")),
    onSuccess: () => {
      queryClient.setQueryData<Me | null>(keys.me, null);
      forgetPrivateData(queryClient);
    },
  });
}

/** Mails a new verification link; the answer is the same whatever the address. */
export function useResendVerification() {
  return useMutation({
    mutationFn: (email: string) => send(api.POST("/api/v1/auth/verification", { body: { email } })),
  });
}

/** Opens a mailed verification link; answers the address it verified. */
export function useVerifyEmail() {
  return useMutation({
    mutationFn: async (token: string) =>
      (await call(api.POST("/api/v1/auth/verify-email", { body: { token } }))).email,
  });
}

/** Asks for a link that sets a new password; the answer is the same whatever the address. */
export function useRequestPasswordReset() {
  return useMutation({
    mutationFn: (email: string) =>
      send(api.POST("/api/v1/auth/password-reset", { body: { email } })),
  });
}

/** Sets a new password with a mailed link's token; answers the account's address. */
export function useResetPassword() {
  return useMutation({
    mutationFn: async (body: Schemas["ResetPasswordRequest"]) =>
      (await call(api.POST("/api/v1/auth/password-reset/confirm", { body }))).email,
  });
}

/**
 * Whether an invite works, and the address it is bound to, if any. Read once
 * per code: nothing about it changes while the form is open.
 */
export function inviteQuery(code: string) {
  return queryOptions({
    queryKey: keys.invite(code),
    queryFn: async ({ signal }) =>
      (await call(api.POST("/api/v1/auth/invite", { body: { code }, signal }))).email ?? null,
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  });
}

/** Asks to join. The answer is the same whatever comes of it. */
export function useRequestToJoin() {
  return useMutation({
    mutationFn: (body: Schemas["JoinRequestRequest"]) =>
      send(api.POST("/api/v1/auth/join-requests", { body })),
  });
}

/** Confirms a request to join with the mailed link's token, so admins see it. */
export function useConfirmJoin() {
  return useMutation({
    mutationFn: (token: string) =>
      send(api.POST("/api/v1/auth/join-requests/confirm", { body: { token } })),
  });
}
