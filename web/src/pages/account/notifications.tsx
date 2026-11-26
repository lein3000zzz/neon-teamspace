import { useQuery } from "@tanstack/react-query";
import {
  notificationSettingsQuery,
  useSetMail,
  type MailPreference,
  type NotificationSettings,
} from "@/api/inbox";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { SettingSection } from "@/components/settings/setting-section";
import { Alert } from "@/components/ui/alert";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { strings } from "@/lib/strings";

const copy = strings.notificationMail;

const CHOICES: readonly MailPreference[] = ["none", "each", "digest"];

/** How often a digest goes at most, in the largest unit that reads well. */
function every(seconds: number): string {
  return seconds >= 3600 && seconds % 3600 === 0
    ? copy.every.hours(seconds / 3600)
    : copy.every.minutes(Math.max(1, Math.round(seconds / 60)));
}

function hintOf(mail: MailPreference, settings: NotificationSettings): string {
  return mail === "digest"
    ? copy.hints.digest(every(settings.digest_interval_secs))
    : copy.hints[mail];
}

/**
 * How the user wants mail of their inbox: none, one for each notification
 * still unread a while after it came, or a digest. The choice is saved as it
 * is made, and holds while the instance sends no notification mail.
 */
export function NotificationsPage() {
  const settings = useQuery(notificationSettingsQuery);
  const save = useSetMail();

  return (
    <SettingSection title={copy.title} hint={copy.hint}>
      {(headingId) => {
        if (settings.isPending) {
          return <Loading />;
        }
        if (settings.isError) {
          return (
            <ErrorNotice
              error={settings.error}
              retrying={settings.isFetching}
              onRetry={() => void settings.refetch()}
            />
          );
        }
        const current = save.isPending ? save.variables : settings.data.mail;
        return (
          <div className="grid gap-3">
            {!settings.data.mail_available && (
              <Alert live="polite">
                <p className="text-sm">{copy.unavailable}</p>
              </Alert>
            )}
            <ToggleGroup
              type="single"
              aria-labelledby={headingId}
              value={current}
              disabled={save.isPending}
              onValueChange={(value) => {
                const mail = CHOICES.find((choice) => choice === value);
                if (mail !== undefined && mail !== settings.data.mail) {
                  save.mutate(mail);
                }
              }}
            >
              {CHOICES.map((choice) => (
                <ToggleGroupItem key={choice} value={choice}>
                  {copy.choices[choice]}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
            <p className="text-sm text-muted-foreground">{hintOf(current, settings.data)}</p>
            {save.isError && <ErrorNotice error={save.error} />}
          </div>
        );
      }}
    </SettingSection>
  );
}
