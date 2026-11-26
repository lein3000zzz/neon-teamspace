import type { TextRun } from "@/api/search";

/**
 * Text in runs, those the query matched marked. Plain text throughout: the
 * server sends words, never markup.
 */
export function Runs({ runs }: { runs: TextRun[] }) {
  return (
    <>
      {placed(runs).map(({ run, at }) =>
        run.highlight ? (
          <mark key={at} className="rounded-sm bg-primary/20 px-0.5 text-foreground">
            {run.text}
          </mark>
        ) : (
          <span key={at}>{run.text}</span>
        ),
      )}
    </>
  );
}

/** Each run with where it starts in the text, which tells it apart from the others. */
function placed(runs: TextRun[]): { run: TextRun; at: number }[] {
  let at = 0;
  return runs.map((run) => {
    const start = at;
    at += run.text.length;
    return { run, at: start };
  });
}
