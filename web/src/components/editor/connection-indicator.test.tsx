import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ConnectionIndicator } from "./connection-indicator";
import { STARTING } from "./connection-state";

describe("connection indicator", () => {
  it("says who else is here, an agent with the person it acts for", () => {
    render(
      <ConnectionIndicator
        readOnly={false}
        connection={{
          ...STARTING,
          status: "connected",
          peers: [
            { id: "0193a5b2-0000-7000-8000-00000000c002", name: "Bob", hue: 30 },
            {
              id: "0193a5b2-0000-7000-8000-00000000c0a1",
              name: "Scout",
              hue: 200,
              kind: "agent",
              owner: "Ada",
            },
          ],
        }}
      />,
    );

    expect(screen.getByText("Also here: Bob, Scout · via Ada")).toBeInTheDocument();
  });
});
