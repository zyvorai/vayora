// Demo: UPI intent deep link (opens the user's UPI app on mobile). Simulator payee only.
export default function Checkout() {
  const amount = "499.00";
  const upi = `upi://pay?pa=simulator@vayora&pn=Vayora%20Demo&am=${amount}&cu=INR`;
  return (
    <main>
      <p className="label">Vayora Demo Store</p>
      <h1 className="amount">₹{amount}</h1>
      <a className="pay" href={upi}>Pay with UPI</a>
      <p className="note">Simulator mode. No real money moves.</p>
    </main>
  );
}
