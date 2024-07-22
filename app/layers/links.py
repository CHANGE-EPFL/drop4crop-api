from sqlmodel import Field, SQLModel
from uuid import UUID


class LayerCountryLink(SQLModel, table=True):
    country_id: UUID | None = Field(
        default=None,
        foreign_key="country.id",
        primary_key=True,
    )
    layer_id: UUID | None = Field(
        default=None,
        foreign_key="layer.id",
        primary_key=True,
    )
    value: float | None = Field(
        default=None,
        nullable=True,
        index=True,
    )
