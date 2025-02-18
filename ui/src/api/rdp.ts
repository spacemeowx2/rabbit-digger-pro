interface Net {
    type: string;
    [key: string]: unknown;
}

export interface SelectNet extends Net {
    type: "select";
    list: string[];
}

export function isSelectNet(net: Net): net is SelectNet {
    return net.type === "select";
}
