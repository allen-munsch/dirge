import React from "react";

function helper(x: number): number {
    return x * 2;
}

export const Component: React.FC = () => {
    const doubled = helper(42);
    return <div>{doubled}</div>;
};
