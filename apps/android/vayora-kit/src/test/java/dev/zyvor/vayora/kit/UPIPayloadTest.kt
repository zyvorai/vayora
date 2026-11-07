package dev.zyvor.vayora.kit

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class UPIPayloadTest {
    @Test
    fun parsesHostileSafePayload() {
        val p = UPIPayload.parse("upi://pay?pa=asha@upi&pn=Asha&am=499.00&cu=INR&tn=att_1")
        assertEquals("asha@upi", p.payee)
        assertEquals("Asha", p.name)
        assertEquals(49900L, p.amount!!.minor)
        assertEquals("att_1", p.note)
    }

    @Test
    fun refusesDuplicateAmountAndBadVpa() {
        try {
            UPIPayload.parse("upi://pay?pa=a@b&am=1&am=2")
            error("expected")
        } catch (e: UPIPayload.Failure.BadAmount) {
            assertTrue(true)
        }
        try {
            UPIPayload.parse("upi://pay?pa=not-a-vpa&am=1")
            error("expected")
        } catch (e: UPIPayload.Failure.BadPayee) {
            assertTrue(true)
        }
    }
}
