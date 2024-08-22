export type Type = "Unit" | "Bool" | "I8" | "I16" | "I32" | "I64" | "I128" | "U8" | "U16" | "U32" | "U64" | "U128" | "String" | {
    Vec: Type;
} | {
    Tuple: Array<Type>;
} | {
    Other: {
        name: string;
    };
};
