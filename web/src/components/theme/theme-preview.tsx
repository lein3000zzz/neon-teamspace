import { Chip, ChipList } from "@/components/nodes/chip";
import { HueDot } from "@/components/nodes/hue-dot";
import { StepFlow, StepNode } from "@/components/nodes/step-node";
import { Button } from "@/components/ui/button";
import { strings } from "@/lib/strings";

/**
 * Every hue-carrying element at once, on the current theme, so a change to the
 * palette, mode or accent shows its effect where it matters.
 */
export function ThemePreview() {
  const copy = strings.appearance;
  return (
    <figure className="m-0 grid gap-4 rounded-lg border border-border bg-background dot-grid p-4">
      <figcaption className="flex items-center gap-2 font-mono text-xs tracking-wider text-muted-foreground">
        <HueDot />
        {copy.preview}
      </figcaption>
      <div className="flex flex-wrap gap-2">
        <Button>{copy.previewPrimary}</Button>
        <Button variant="pill">{copy.previewPill}</Button>
        <Button variant="outline">{copy.previewOutline}</Button>
      </div>
      <ChipList>
        <Chip tone="strong">{copy.previewChip}</Chip>
        <Chip>{copy.palettes.nothing}</Chip>
        <Chip>{copy.palettes.soft}</Chip>
        <Chip>{copy.palettes.warm}</Chip>
      </ChipList>
      <StepFlow>
        <StepNode title={copy.palette} />
        <StepNode title={copy.mode} />
        <StepNode title={copy.accent} active />
      </StepFlow>
    </figure>
  );
}
