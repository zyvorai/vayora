package dev.zyvor.vayora.kit

import kotlinx.serialization.json.Json

internal val json = Json {
    ignoreUnknownKeys = true
    isLenient = true
    encodeDefaults = false
}
