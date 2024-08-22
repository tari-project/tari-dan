import type { TemplateAbi } from "./TemplateAbi";
import type { TemplateMetadata } from "./TemplateMetadata";
export interface GetTemplateResponse {
    registration_metadata: TemplateMetadata;
    abi: TemplateAbi;
}
